// join_user.rs - RACE CONDITION FIXED WITH ATOMIC OPERATIONS
use crate::{errors::WagerError, state::*, utils::validate_session_id, TOKEN_ID};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn join_user_handler(ctx: Context<JoinUser>, session_id: String, team: u8) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;

    // CRITICAL FIX: Enhanced validation
    validate_session_id(&session_id)?;

    // CRITICAL FIX: Atomic validation - only allow joining from WaitingForPlayers
    require!(
        game_session.status == GameStatus::WaitingForPlayers,
        WagerError::InvalidGameState
    );

    // Validate team number (0 for team A, 1 for team B)
    require!(team == 0 || team == 1, WagerError::InvalidTeamSelection);

    // CRITICAL FIX: Check for duplicate players across both teams BEFORE any operations
    let player_key = ctx.accounts.user.key();
    require!(
        !game_session.is_player_already_joined(player_key)?,
        WagerError::DuplicatePlayer
    );

    // Check if team is full already
    let empty_index = game_session.get_player_empty_slot(team)?;

    let session_bet = game_session.session_bet;

    // CRITICAL FIX: Enhanced balance validation
    require!(
        ctx.accounts.user_token_account.amount >= session_bet,
        WagerError::InsufficientFunds
    );

    // Add buffer check to ensure user doesn't spend their last tokens
    let min_buffer = 1000; // Keep some tokens for fees
    require!(
        ctx.accounts.user_token_account.amount >= session_bet.saturating_add(min_buffer),
        WagerError::InsufficientFunds
    );

    // CRITICAL FIX: Enhanced token account validation
    require!(
        ctx.accounts.user_token_account.owner == ctx.accounts.user.key(),
        WagerError::InvalidPlayerTokenAccount
    );

    require!(
        ctx.accounts.user_token_account.mint == TOKEN_ID,
        WagerError::InvalidTokenMint
    );

    // CRITICAL FIX: Start atomic operation - set operation lock
    let clock = Clock::get()?;
    let current_time = clock.unix_timestamp;

    // Check if any operation is currently in progress
    if let Some(ref current_op) = game_session.current_operation {
        // Check if operation has timed out (2 minutes for join operations)
        if current_time - game_session.operation_started_at > 120 {
            msg!("Join operation {} timed out, clearing lock", current_op);
            game_session.current_operation = None;
        } else {
            return Err(error!(WagerError::ConcurrentOperation));
        }
    }

    // Set operation lock
    game_session.current_operation = Some(format!(
        "join_user_{}",
        player_key.to_string()[0..8].to_string()
    ));
    game_session.operation_started_at = current_time;

    // Transfer SPL tokens from user to vault using user's signature
    let transfer_result = anchor_spl::token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            anchor_spl::token::Transfer {
                from: ctx.accounts.user_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
        session_bet,
    );

    // Handle transfer failure - clear operation lock
    if let Err(e) = transfer_result {
        game_session.current_operation = None;
        game_session.operation_started_at = 0;
        return Err(e.into());
    }

    // Get reference to the selected team
    let selected_team = if team == 0 {
        &mut game_session.team_a
    } else {
        &mut game_session.team_b
    };

    // CRITICAL FIX: Double-check empty slot is still available (race condition protection)
    require!(
        selected_team.players[empty_index] == Pubkey::default(),
        WagerError::ConcurrentOperation
    );

    // Add player to the first available slot atomically
    selected_team.players[empty_index] = player_key;
    selected_team.player_spawns[empty_index] = 10;
    selected_team.player_kills[empty_index] = 0;

    // CRITICAL FIX: Enhanced bet tracking with overflow protection
    selected_team.total_bet = selected_team
        .total_bet
        .checked_add(session_bet)
        .ok_or(WagerError::ArithmeticError)?;

    // CRITICAL FIX: Additional validation - ensure total bet doesn't exceed safe limits
    require!(
        selected_team.total_bet <= crate::state::MAX_TEAM_BET,
        WagerError::ArithmeticError
    );

    // Update operation tracking
    game_session.update_operation_tracking()?;

    // Check if both teams are full and update status atomically
    if game_session.check_all_filled()? {
        let current_nonce = game_session.nonce; // Extract nonce before mutable borrow
        let success = game_session.compare_and_swap_status(
            GameStatus::WaitingForPlayers,
            current_nonce, // Pass the current nonce
            GameStatus::InProgress,
            Some("auto_start_game"),
        )?;

        if success {
            msg!("Game automatically started - both teams full");
        } else {
            // Another thread might have started the game - that's okay
            msg!("Game start handled by another operation");
        }
    } else {
        // Clear operation lock since we're done with player addition
        game_session.current_operation = None;
        game_session.operation_started_at = 0;
    }

    msg!(
        "Player {} joined team {} at position {} with bet {}",
        player_key,
        team,
        empty_index,
        session_bet
    );

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct JoinUser<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    /// CHECK: Game server authority needed for PDA derivation - not required to be signer for join operation
    pub game_server: AccountInfo<'info>,

    // UPDATED: Enhanced PDA seeds with authority
    #[account(
        mut,
        seeds = [
            b"game_session", 
            session_id.as_bytes(),
            game_server.key().as_ref()
        ],
        bump = game_session.bump,
    )]
    pub game_session: Account<'info, GameSession>,
    /// CHECK: This is a PDA (Program Derived Address) used as the authority for token transfers.
    /// It's validated through the seeds constraint which ensures it's derived from the correct
    /// session_id and game_server. The PDA serves as a secure vault authority and doesn't need
    /// additional type validation since it's only used for signing token transfers, not data access.

    #[account(
        mut,
        constraint = user_token_account.owner == user.key(),
        constraint = user_token_account.mint == TOKEN_ID
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    /// CHECK: Vault PDA derived from session_id and authority - used only for token transfers
    #[account(
        mut,
        seeds = [
            b"vault", 
            session_id.as_bytes(),
            game_server.key().as_ref()
        ],
        bump = game_session.vault_bump,
    )]
    pub vault: AccountInfo<'info>,

    #[account(
        mut,
        associated_token::mint = mint,
        associated_token::authority = vault,
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        mut,
        address = TOKEN_ID @ WagerError::InvalidMint
    )]
    pub mint: Account<'info, anchor_spl::token::Mint>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
