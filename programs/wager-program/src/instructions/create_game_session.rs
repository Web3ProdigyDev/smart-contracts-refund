use crate::{
    errors::WagerError,
    state::*,
    utils::{generate_session_hash, validate_bet_amount, validate_session_id},
    TOKEN_ID,
};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn create_game_session_handler(
    ctx: Context<CreateGameSession>,
    session_id: String,
    bet_amount: u64,
    game_mode: GameMode,
) -> Result<()> {
    // Validate inputs
    validate_session_id(&session_id)?;
    validate_bet_amount(bet_amount)?;

    // Generate session hash for collision prevention
    let clock = Clock::get()?;
    let session_hash = generate_session_hash(
        &session_id,
        ctx.accounts.game_server.key(),
        clock.unix_timestamp,
    );

    // Validate game server authority
    require!(
        ctx.accounts.game_server.key() != Pubkey::default()
            && ctx.accounts.game_server.key() != anchor_lang::system_program::ID,
        WagerError::AuthorityMismatch
    );

    // Validate mint
    require!(ctx.accounts.mint.key() == TOKEN_ID, WagerError::InvalidMint);
    require!(
        ctx.accounts.mint.is_initialized,
        WagerError::InvalidTokenMint
    );

    // Validate bet amount with overflow protection
    require!(bet_amount <= u64::MAX / 20, WagerError::InvalidBetAmount);

    // Validate game mode
    let players_per_team = game_mode.players_per_team();
    require!(
        players_per_team >= 1 && players_per_team <= 5,
        WagerError::InvalidGameModeForOperation
    );

    // Calculate and validate total pot size
    let max_possible_pot = bet_amount
        .checked_mul(players_per_team as u64)
        .and_then(|v| v.checked_mul(2))
        .ok_or(WagerError::ArithmeticError)?;
    const MAX_TOTAL_POT: u64 = 10_000_000_000_000;
    require!(
        max_possible_pot <= MAX_TOTAL_POT,
        WagerError::InvalidBetAmount
    );

    // Store immutable data before mutable borrow
    let game_session_key = ctx.accounts.game_session.key();
    let vault_key = ctx.accounts.vault.key();
    let vault_token_account_key = ctx.accounts.vault_token_account.key();

    // Now create mutable borrow
    let game_session = &mut ctx.accounts.game_session;

    // Initialize game session
    game_session.initialize(
        session_id.clone(),
        ctx.accounts.game_server.key(),
        bet_amount,
        game_mode,
        ctx.bumps.game_session,
        ctx.bumps.vault,
    )?;
    game_session.validate_integrity()?;

    // Store created_at after initialization
    let created_at = game_session.created_at;

    // Validate vault token account
    require!(
        ctx.accounts.vault_token_account.mint == TOKEN_ID
            && ctx.accounts.vault_token_account.owner == ctx.accounts.vault.key()
            && ctx.accounts.vault_token_account.amount == 0,
        WagerError::InvalidPlayerTokenAccount
    );

    // Consolidated logging
    #[cfg(not(feature = "no-log"))]
    msg!(
        "Game session created: ID={}, Hash={:?}, Bet={}, Mode={:?}, PlayersPerTeam={}, MaxPot={}, Authority={}, CreatedAt={}, SessionPDA={}, VaultPDA={}, VaultToken={}",
        session_id,
        session_hash,
        bet_amount,
        game_mode,
        players_per_team,
        max_possible_pot,
        ctx.accounts.game_server.key(),
        created_at,
        game_session_key,
        vault_key,
        vault_token_account_key
    );

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct CreateGameSession<'info> {
    #[account(
        mut,
        constraint = game_server.key() != Pubkey::default() @ WagerError::AuthorityMismatch,
    )]
    pub game_server: Signer<'info>,
    #[account(
        init,
        payer = game_server,
        space = GameSession::MAX_SIZE,
        seeds = [
            b"game_session",
            session_id.as_bytes(),
            game_server.key().as_ref(),
        ],
        bump,
        constraint = session_id.len() >= 12 && session_id.len() <= 32 @ WagerError::InvalidSessionId,
    )]
    pub game_session: Account<'info, GameSession>,
    /// CHECK: This is a PDA (Program Derived Address) used as the authority for token transfers.
    /// It's validated through the seeds constraint which ensures it's derived from the correct
    /// session_id and game_server. The PDA serves as a secure vault authority and doesn't need
    /// additional type validation since it's only used for signing token transfers, not data access.
    #[account(
        init,
        payer = game_server,
        space = 0,
        seeds = [
            b"vault",
            session_id.as_bytes(),
            game_server.key().as_ref(),
        ],
        bump,
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
        constraint = mint.key() == TOKEN_ID @ WagerError::InvalidMint,
        constraint = mint.is_initialized @ WagerError::InvalidTokenMint,
    )]
    pub mint: Account<'info, anchor_spl::token::Mint>,
    #[account(
        constraint = token_program.key() == anchor_spl::token::ID @ WagerError::InvalidTokenProgram,
    )]
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
    pub rent: Sysvar<'info, Rent>,
}
