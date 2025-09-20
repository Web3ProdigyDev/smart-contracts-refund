// pay_to_spawn.rs - SECURITY HARDENED VERSION WITH ARITHMETIC FIXES
use crate::{errors::WagerError, state::*, TOKEN_ID, utils::validate_session_id};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn pay_to_spawn_handler(ctx: Context<PayToSpawn>, session_id: String, team: u8) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    
    // CRITICAL FIX: Validate session_id format
    validate_session_id(&session_id)?;
    
    // CRITICAL FIX: Check game hasn't expired with safe arithmetic
    game_session.validate_not_expired_safe()?;

    // Check if game status is in progress and if it is a pay to spawn game
    require!(
        game_session.status == GameStatus::InProgress && game_session.is_pay_to_spawn(),
        WagerError::InvalidGameState
    );

    // Validate team number (0 for team A, 1 for team B)
    require!(team == 0 || team == 1, WagerError::InvalidTeamSelection);

    // CRITICAL FIX: Verify user is actually in the specified team
    let player_index = game_session.get_player_index(team, ctx.accounts.user.key())?;
    
    // CRITICAL FIX: Check current spawn count before allowing purchase with safe limits
    let current_spawns = match team {
        0 => game_session.team_a.player_spawns[player_index],
        1 => game_session.team_b.player_spawns[player_index],
        _ => return Err(error!(WagerError::InvalidTeam)),
    };
    
    // CRITICAL FIX: Enhanced spawn limit validation with overflow protection
    const MAX_SPAWNS: u8 = 100;
    const SPAWN_INCREMENT: u8 = 10;
    
    // Check if adding spawn increment would exceed limit or overflow
    require!(
        current_spawns <= MAX_SPAWNS.saturating_sub(SPAWN_INCREMENT),
        WagerError::SpawnLimitExceeded
    );
    
    // Double-check with checked_add to prevent any overflow scenario
    require!(
        current_spawns.checked_add(SPAWN_INCREMENT).is_some(),
        WagerError::ArithmeticError
    );

    let session_bet = game_session.session_bet;
    
    // CRITICAL FIX: Verify user has sufficient balance with safe comparison
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

    // CRITICAL FIX: Add spawns BEFORE token transfer (checks-effects-interactions pattern)
    // Using the safe add_spawns_safe method
    game_session.add_spawns_safe(team, player_index)?;

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

    msg!("Player {} purchased {} spawns for team {} (total spawns now: {})", 
         ctx.accounts.user.key(), SPAWN_INCREMENT, team, 
         match team {
             0 => game_session.team_a.player_spawns[player_index],
             1 => game_session.team_b.player_spawns[player_index],
             _ => 0,
         });

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct PayToSpawn<'info> {
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
        associated_token::mint = TOKEN_ID,
        associated_token::authority = vault,
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}