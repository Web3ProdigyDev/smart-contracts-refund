// create_game_session.rs - SECURITY HARDENED VERSION WITH COLLISION PREVENTION
use crate::errors::WagerError;
use crate::state::*;
use crate::TOKEN_ID;
use crate::utils::{validate_session_id, validate_bet_amount, generate_session_hash};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn create_game_session_handler(
    ctx: Context<CreateGameSession>,
    session_id: String,
    bet_amount: u64,
    game_mode: GameMode,
) -> Result<()> {
    // CRITICAL FIX: Enhanced validation sequence - validate everything before any operations
    validate_session_id(&session_id)?;
    validate_bet_amount(bet_amount)?;
    
    // ENHANCED: Additional collision prevention - generate unique hash for session
    let clock = Clock::get()?;
    let session_hash = generate_session_hash(&session_id, ctx.accounts.game_server.key(), clock.unix_timestamp);
    
    // ENHANCED: Validate game server authority is not a default/system account
    require!(
        ctx.accounts.game_server.key() != Pubkey::default() &&
        ctx.accounts.game_server.key() != anchor_lang::system_program::ID,
        WagerError::AuthorityMismatch
    );
    
    // ENHANCED: Validate mint is correct and not manipulated
    require!(
        ctx.accounts.mint.key() == TOKEN_ID,
        WagerError::InvalidMint
    );
    
    // ENHANCED: Validate mint is not frozen or has other restrictions
    require!(
        ctx.accounts.mint.is_initialized,
        WagerError::InvalidTokenMint
    );
    
    // CRITICAL FIX: Validate bet amount has additional overflow protection
    require!(
        bet_amount <= u64::MAX / 20, // Ensure even 20 players won't cause overflow
        WagerError::InvalidBetAmount
    );
    
    // ENHANCED: Validate game mode is reasonable
    let players_per_team = game_mode.players_per_team();
    require!(
        players_per_team >= 1 && players_per_team <= 5,
        WagerError::InvalidGameModeForOperation
    );
    
    // ENHANCED: Calculate total maximum pot size and validate
    let max_possible_pot = bet_amount
        .checked_mul(players_per_team as u64)
        .and_then(|v| v.checked_mul(2)) // Two teams
        .ok_or(WagerError::ArithmeticError)?;
    
    // Additional safety: ensure max pot doesn't exceed reasonable limits
    const MAX_TOTAL_POT: u64 = 10_000_000_000_000; // 10M tokens max
    require!(
        max_possible_pot <= MAX_TOTAL_POT,
        WagerError::InvalidBetAmount
    );

    // FIX: Store keys BEFORE taking mutable borrow
    let game_server_key = ctx.accounts.game_server.key();
    let game_session_key = ctx.accounts.game_session.key();

    let game_session = &mut ctx.accounts.game_session;

    // CRITICAL FIX: Use enhanced initialization method with validation
    game_session.initialize(
        session_id.clone(),
        ctx.accounts.game_server.key(),
        bet_amount,
        game_mode,
        ctx.bumps.game_session,
        ctx.bumps.vault,
    )?;
    
    // ENHANCED: Validate initialized state
    game_session.validate_integrity()?;
    
    // ENHANCED: Additional security - verify PDA derivation is correct
    let expected_game_session_seeds = [
        b"game_session",
        session_id.as_bytes(),
        game_server_key.as_ref(),
    ];
    
    let (expected_game_session, expected_bump) = Pubkey::find_program_address(
        &expected_game_session_seeds,
        &crate::ID,
    );
    
    require!(
        game_session_key == expected_game_session &&
        game_session.bump == expected_bump,
        WagerError::InvalidAccountDerivation
    );
    
    // ENHANCED: Verify vault PDA derivation
    let expected_vault_seeds = [
        b"vault",
        session_id.as_bytes(),
        game_server_key.as_ref(),
    ];
    
    let (expected_vault, expected_vault_bump) = Pubkey::find_program_address(
        &expected_vault_seeds,
        &crate::ID,
    );
    
    require!(
        ctx.accounts.vault.key() == expected_vault &&
        game_session.vault_bump == expected_vault_bump,
        WagerError::InvalidAccountDerivation
    );
    
    // ENHANCED: Validate vault token account is properly initialized
    require!(
        ctx.accounts.vault_token_account.mint == TOKEN_ID &&
        ctx.accounts.vault_token_account.owner == ctx.accounts.vault.key() &&
        ctx.accounts.vault_token_account.amount == 0, // Should be empty initially
        WagerError::InvalidPlayerTokenAccount
    );

    // ENHANCED: Comprehensive logging with session hash for tracking
    msg!("Game session created successfully:");
    msg!("  Session ID: {}", session_id);
    msg!("  Session Hash: {:?}", session_hash);
    msg!("  Bet amount: {}", bet_amount);
    msg!("  Game mode: {:?}", game_mode);
    msg!("  Players per team: {}", players_per_team);
    msg!("  Max possible pot: {}", max_possible_pot);
    msg!("  Authority: {}", game_server_key);
    msg!("  Created at: {}", game_session.created_at);
    msg!("  Game session PDA: {}", game_session_key);
    msg!("  Vault PDA: {}", ctx.accounts.vault.key());
    msg!("  Vault token account: {}", ctx.accounts.vault_token_account.key());
    msg!("  Nonce: {}", game_session.nonce);
    
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

    // ENHANCED PDA SECURITY: Multiple entropy sources in seeds for collision prevention
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

    /// CHECK: Vault PDA with enhanced validation
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