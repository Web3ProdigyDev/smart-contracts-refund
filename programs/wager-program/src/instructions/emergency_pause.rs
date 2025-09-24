use crate::errors::WagerError;
use crate::state::{GameSession, GameStatus};
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct EmergencyPause<'info> {
    #[account(mut)]
    pub game_session: Account<'info, GameSession>,
    #[account(
        constraint = authority.key() == game_session.authority @ WagerError::UnauthorizedDistribution
    )]
    pub authority: Signer<'info>,
}

pub fn emergency_pause(ctx: Context<EmergencyPause>) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let clock = Clock::get()?;

    require!(
        !game_session.status.is_final(),
        WagerError::InvalidGameState
    );

    let current_status = game_session.status.clone();
    let current_nonce = game_session.nonce; // Extract nonce before mutable borrow
    let success = game_session.compare_and_swap_status(
        current_status,
        current_nonce,
        GameStatus::EmergencyPaused,
        Some("emergency_pause"),
    )?;

    require!(success, WagerError::ConcurrentOperation);

    game_session.last_operation = clock.unix_timestamp;
    msg!(
        "Game session {} paused at {}",
        game_session.session_id,
        clock.unix_timestamp
    );
    Ok(())
}
