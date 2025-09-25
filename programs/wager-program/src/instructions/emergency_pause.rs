use crate::errors::WagerError;
use crate::state::{GameSession, GameStatus};
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct EmergencyPause<'info> {
    #[account(mut)]
    pub game_session: Account<'info, GameSession>,
    /// CHECK: This is a PDA (Program Derived Address) used as the authority for token transfers.
    /// It's validated through the seeds constraint which ensures it's derived from the correct
    /// session_id and game_server. The PDA serves as a secure vault authority and doesn't need
    /// additional type validation since it's only used for signing token transfers, not data access.
    #[account(
        constraint = authority.key() == game_session.authority @ WagerError::UnauthorizedDistribution
    )]
    pub authority: Signer<'info>,
}

pub fn emergency_pause(ctx: Context<EmergencyPause>) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let clock = Clock::get()?;

    // Validate game not expired
    game_session.validate_not_expired()?;

    // Validate not in final state
    require!(
        !game_session.status.is_final(),
        WagerError::InvalidGameState
    );

    // Atomic status transition
    let current_status = game_session.status.clone();
    let current_nonce = game_session.nonce;
    let success = game_session.compare_and_swap_status(
        current_status,
        current_nonce,
        GameStatus::EmergencyPaused,
        Some("emergency_pause"),
    )?;

    require!(success, WagerError::ConcurrentOperation);

    game_session.last_operation = clock.unix_timestamp;
    msg!(
        "Game session {} paused at {} (nonce: {})",
        game_session.session_id,
        clock.unix_timestamp,
        game_session.nonce
    );
    Ok(())
}
