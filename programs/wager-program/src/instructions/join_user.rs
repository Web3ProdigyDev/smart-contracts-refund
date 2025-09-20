// join_user.rs - SECURITY HARDENED VERSION
use crate::{errors::WagerError, state::*, TOKEN_ID, utils::validate_session_id};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn join_user_handler(ctx: Context<JoinUser>, session_id: String, team: u8) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    
    // CRITICAL FIX: Validate session_id format
    validate_session_id(&session_id)?;

    // Validate game status
    require!(
        game_session.status == GameStatus::WaitingForPlayers,
        WagerError::InvalidGameState
    );

    // Validate team number (0 for team A, 1 for team B)
    require!(team == 0 || team == 1, WagerError::InvalidTeamSelection);

    // CRITICAL FIX: Check for duplicate players across both teams
    let player_key = ctx.accounts.user.key();
    require!(
        !game_session.is_player_already_joined(player_key)?,
        WagerError::DuplicatePlayer
    );

    // Check if team is full already
    let empty_index = game_session.get_player_empty_slot(team)?;

    let session_bet = game_session.session_bet;
    
    // CRITICAL FIX: Verify user has sufficient balance before transfer
    require!(
        ctx.accounts.user_token_account.amount >= session_bet,
        WagerError::InsufficientFunds
    );

    // CRITICAL FIX: Verify token account ownership and mint
    require!(
        ctx.accounts.user_token_account.owner == ctx.accounts.user.key(),
        WagerError::InvalidPlayerTokenAccount
    );
    
    require!(
        ctx.accounts.user_token_account.mint == TOKEN_ID,
        WagerError::InvalidTokenMint
    );

    // Transfer SPL tokens from user to vault using user's signature
    anchor_spl::token::transfer(
        CpiContext::new(
            ctx.accounts.token_program.to_account_info(),
            anchor_spl::token::Transfer {
                from: ctx.accounts.user_token_account.to_account_info(),
                to: ctx.accounts.vault_token_account.to_account_info(),
                authority: ctx.accounts.user.to_account_info(),
            },
        ),
        session_bet,
    )?;

    // Get reference to the selected team
    let selected_team = if team == 0 {
        &mut game_session.team_a
    } else {
        &mut game_session.team_b
    };

    // Add player to the first available slot
    selected_team.players[empty_index] = player_key;
    selected_team.player_spawns[empty_index] = 10;
    selected_team.player_kills[empty_index] = 0;
    
    // CRITICAL FIX: Update team's total bet tracking
    selected_team.total_bet = selected_team.total_bet
        .checked_add(session_bet)
        .ok_or(WagerError::ArithmeticError)?;

    // Check if both teams are full and update status atomically
    if game_session.check_all_filled()? {
        game_session.status = GameStatus::InProgress;
    }

    msg!("Player {} joined team {} at position {}", player_key, team, empty_index);

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct JoinUser<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    /// CHECK: Game server authority
    pub game_server: AccountInfo<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
    )]
    pub game_session: Account<'info, GameSession>,

    #[account(
        mut,
        constraint = user_token_account.owner == user.key(),
        constraint = user_token_account.mint == TOKEN_ID
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    /// CHECK: Vault PDA that holds the funds
    #[account(
        mut,
        seeds = [b"vault", session_id.as_bytes()],
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