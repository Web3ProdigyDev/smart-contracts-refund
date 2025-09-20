// pay_to_spawn.rs - ATOMIC OPERATIONS WITH COMPREHENSIVE SAFETY CHECKS
use crate::{errors::WagerError, state::*, TOKEN_ID, utils::{validate_session_id, validate_bet_amount, validate_spawn_count}};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn pay_to_spawn_handler(ctx: Context<PayToSpawn>, session_id: String, team: u8) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    
    // CRITICAL FIX: Enhanced validation sequence - all validations before any state changes
    validate_session_id(&session_id)?;
    
    // CRITICAL FIX: Comprehensive game state validation with integrity check
    game_session.validate_integrity()?;
    game_session.validate_not_expired_safe()?;

    // Enhanced status validation - only allow operations during active game
    require!(
        game_session.status.allows_game_operations() && game_session.is_pay_to_spawn(),
        WagerError::InvalidGameState
    );

    // Validate team number (0 for team A, 1 for team B)
    require!(team == 0 || team == 1, WagerError::InvalidTeamSelection);

    // CRITICAL FIX: Enhanced player verification with comprehensive bounds checking
    let player_key = ctx.accounts.user.key();
    require!(player_key != Pubkey::default(), WagerError::InvalidPlayer);
    
    let player_index = game_session.get_player_index(team, player_key)?;
    
    // Validate player index is within bounds for the game mode
    let max_players = game_session.game_mode.players_per_team();
    require!(
        player_index < max_players,
        WagerError::PlayerIndexOutOfBounds
    );
    
    // ATOMIC OPERATION: Get current spawn count and validate increment atomically
    let current_spawns = match team {
        0 => game_session.team_a.player_spawns[player_index],
        1 => game_session.team_b.player_spawns[player_index],
        _ => return Err(error!(WagerError::InvalidTeam)),
    };
    
    // CRITICAL FIX: Use utility function for comprehensive spawn validation
    const SPAWN_INCREMENT: u8 = 10;
    validate_spawn_count(current_spawns, SPAWN_INCREMENT)?;

    let session_bet = game_session.session_bet;
    validate_bet_amount(session_bet)?; // Additional validation
    
    // CRITICAL FIX: Enhanced financial validation with comprehensive buffer checks
    let user_balance = ctx.accounts.user_token_account.amount;
    require!(
        user_balance >= session_bet,
        WagerError::InsufficientFunds
    );
    
    // Enhanced buffer validation - ensure user keeps reasonable amount for future operations
    let min_buffer = std::cmp::max(session_bet / 10, 1000u64); // At least 10% or 1000 tokens
    require!(
        user_balance >= session_bet.saturating_add(min_buffer),
        WagerError::InsufficientFunds
    );
    
    // CRITICAL FIX: Comprehensive token account validation (before state changes)
    require!(
        ctx.accounts.user_token_account.owner == ctx.accounts.user.key(),
        WagerError::InvalidPlayerTokenAccount
    );
    
    require!(
        ctx.accounts.user_token_account.mint == TOKEN_ID,
        WagerError::InvalidTokenMint
    );

    // ENHANCED: Validate vault token account state before transfer
    require!(
        ctx.accounts.vault_token_account.mint == TOKEN_ID &&
        ctx.accounts.vault_token_account.owner == ctx.accounts.vault.key(),
        WagerError::InvalidPlayerTokenAccount
    );

    // CRITICAL FIX: ATOMIC STATE UPDATE - Update spawns FIRST (before token transfer)
    // This ensures state consistency even if token transfer fails
    let original_spawns = current_spawns;
    
    // Use the safe spawn addition method which includes all validations
    game_session.add_spawns_safe(team, player_index)?;
    
    // Verify spawn addition was successful (atomic verification)
    let new_spawns = match team {
        0 => game_session.team_a.player_spawns[player_index],
        1 => game_session.team_b.player_spawns[player_index],
        _ => return Err(error!(WagerError::InvalidTeam)),
    };
    
    require!(
        new_spawns == original_spawns + SPAWN_INCREMENT,
        WagerError::GameDataCorruption
    );

    msg!("Spawn purchase validated - Player {} on team {} purchasing {} spawns (from {} to {})", 
         player_key, team, SPAWN_INCREMENT, original_spawns, new_spawns);

    // CRITICAL FIX: Enhanced vault balance tracking for atomic verification
    let vault_balance_before = ctx.accounts.vault_token_account.amount;
    let user_balance_before = ctx.accounts.user_token_account.amount;
    
    // ATOMIC TOKEN TRANSFER with comprehensive error handling
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

    // Handle transfer failure - revert spawn addition if transfer fails
    if let Err(e) = transfer_result {
        // CRITICAL: Revert spawn addition on transfer failure
        match team {
            0 => game_session.team_a.player_spawns[player_index] = original_spawns,
            1 => game_session.team_b.player_spawns[player_index] = original_spawns,
            _ => {},
        }
        return Err(e.into());
    }
    
    // CRITICAL FIX: Post-transfer atomic verification
    ctx.accounts.vault_token_account.reload()?;
    ctx.accounts.user_token_account.reload()?;
    
    let vault_balance_after = ctx.accounts.vault_token_account.amount;
    let user_balance_after = ctx.accounts.user_token_account.amount;
    
    // Comprehensive balance verification
    require!(
        vault_balance_after == vault_balance_before
            .checked_add(session_bet)
            .ok_or(WagerError::ArithmeticError)?,
        WagerError::VaultBalanceMismatch
    );
    
    require!(
        user_balance_after == user_balance_before
            .checked_sub(session_bet)
            .ok_or(WagerError::ArithmeticError)?,
        WagerError::TokenTransferFailed
    );

    // ENHANCED: Update team total bet tracking with overflow protection
    match team {
        0 => {
            game_session.team_a.total_bet = game_session.team_a.total_bet
                .checked_add(session_bet)
                .ok_or(WagerError::ArithmeticError)?;
        },
        1 => {
            game_session.team_b.total_bet = game_session.team_b.total_bet
                .checked_add(session_bet)
                .ok_or(WagerError::ArithmeticError)?;
        },
        _ => return Err(error!(WagerError::InvalidTeam)),
    }

    // Final comprehensive logging
    msg!("Spawn purchase completed successfully:");
    msg!("  Player: {}", player_key);
    msg!("  Team: {}", team);
    msg!("  Spawns purchased: {}", SPAWN_INCREMENT);
    msg!("  Total spawns now: {}", new_spawns);
    msg!("  Cost: {}", session_bet);
    msg!("  Vault balance: {} -> {}", vault_balance_before, vault_balance_after);
    msg!("  User balance: {} -> {}", user_balance_before, user_balance_after);
    msg!("  Session nonce: {}", game_session.nonce);

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct PayToSpawn<'info> {
    #[account(
        mut,
        constraint = user.key() != Pubkey::default() @ WagerError::InvalidPlayer,
    )]
    pub user: Signer<'info>,

    /// CHECK: Game server authority needed for PDA derivation - validated through PDA constraints
    #[account(
        constraint = game_server.key() != Pubkey::default() @ WagerError::AuthorityMismatch,
    )]
    pub game_server: AccountInfo<'info>,

    // ENHANCED: PDA with comprehensive validation
    #[account(
        mut,
        seeds = [
            b"game_session", 
            session_id.as_bytes(),
            game_server.key().as_ref()
        ],
        bump = game_session.bump,
        constraint = game_session.session_id == session_id @ WagerError::InvalidSessionId,
        constraint = game_session.authority == game_server.key() @ WagerError::AuthorityMismatch,
    )]
    pub game_session: Account<'info, GameSession>,

    #[account(
        mut,
        constraint = user_token_account.owner == user.key() @ WagerError::InvalidPlayerTokenAccount,
        constraint = user_token_account.mint == TOKEN_ID @ WagerError::InvalidTokenMint,
    )]
    pub user_token_account: Account<'info, TokenAccount>,

    /// CHECK: Vault PDA with enhanced validation
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
        associated_token::mint = TOKEN_ID,
        associated_token::authority = vault,
        constraint = vault_token_account.mint == TOKEN_ID @ WagerError::InvalidTokenMint,
        constraint = vault_token_account.owner == vault.key() @ WagerError::InvalidPlayerTokenAccount,
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        constraint = token_program.key() == anchor_spl::token::ID @ WagerError::InvalidTokenProgram,
    )]
    pub token_program: Program<'info, Token>,
    
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}