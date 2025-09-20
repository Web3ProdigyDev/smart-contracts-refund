// create_game_session.rs - SECURITY HARDENED VERSION
use crate::errors::WagerError;
use crate::state::*;
use crate::TOKEN_ID;
use crate::utils::validate_session_id;
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn create_game_session_handler(
    ctx: Context<CreateGameSession>,
    session_id: String,
    bet_amount: u64,
    game_mode: GameMode,
) -> Result<()> {
    // CRITICAL FIX: Validate session_id format before any other operations
    validate_session_id(&session_id)?;
    
    // CRITICAL FIX: Validate bet amount is reasonable (min 1000, max 1M tokens)
    require!(
        bet_amount >= 1000 && bet_amount <= 1_000_000_000_000, // 1M tokens with 6 decimals
        WagerError::InvalidBetAmount
    );
    
    let clock = Clock::get()?;
    let game_session = &mut ctx.accounts.game_session;

    // CRITICAL FIX: Initialize with proper validation
    game_session.session_id = session_id.clone();
    game_session.authority = ctx.accounts.game_server.key();
    game_session.session_bet = bet_amount;
    game_session.game_mode = game_mode;
    game_session.status = GameStatus::WaitingForPlayers;
    game_session.created_at = clock.unix_timestamp;
    game_session.bump = ctx.bumps.game_session;
    game_session.vault_bump = ctx.bumps.vault;
    
    // CRITICAL FIX: Initialize teams with default values explicitly
    game_session.team_a = Team::default();
    game_session.team_b = Team::default();

    msg!("Game session created with ID: {}", session_id);
    msg!("Bet amount: {}", bet_amount);
    msg!("Game mode: {:?}", game_mode);
    msg!("Game session: {}", game_session.key());
    msg!("Vault: {}", ctx.accounts.vault.key());
    msg!("Vault token account: {}", ctx.accounts.vault_token_account.key());
    
    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct CreateGameSession<'info> {
    #[account(mut)]
    pub game_server: Signer<'info>,

    #[account(
        init,
        payer = game_server,
        space = GameSession::MAX_SIZE,
        seeds = [b"game_session", session_id.as_bytes()],
        bump
    )]
    pub game_session: Account<'info, GameSession>,

    /// CHECK: This is safe as it's just used to store SOL
    #[account(
        init,
        payer = game_server,
        space = 0,
        seeds = [b"vault", session_id.as_bytes()],
        bump
    )]
    pub vault: AccountInfo<'info>,

    #[account(
        init,
        payer = game_server,
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
    pub rent: Sysvar<'info, Rent>,
}