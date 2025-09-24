use crate::{errors::WagerError, state::*};
use anchor_lang::prelude::*;

#[derive(Accounts)]
pub struct EmergencyPause<'info> {
    #[account(mut)]
    pub game_session: Account<'info, GameSession>,
    pub authority: Signer<'info>,
}

pub fn emergency_pause(ctx: Context<EmergencyPause>) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    require!(
        ctx.accounts.authority.key() == game_session.authority,
        WagerError::UnauthorizedPause
    );

    // Store status before mutable operation
    let current_status = game_session.status.clone();

    // Perform status update
    game_session.compare_and_swap_status(
        current_status,
        GameStatus::EmergencyPaused,
        Some("emergency_pause"),
    )?;

    msg!("Game session paused: {}", game_session.session_id);
    Ok(())
}
