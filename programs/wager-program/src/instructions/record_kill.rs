// record_kill.rs - SECURITY HARDENED VERSION
use crate::{errors::WagerError, state::*, utils::validate_session_id};
use anchor_lang::prelude::*;

pub fn record_kill_handler(
    ctx: Context<RecordKill>,
    session_id: String,
    killer_team: u8,
    killer: Pubkey,
    victim_team: u8,
    victim: Pubkey,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    
    // CRITICAL FIX: Validate session_id format
    validate_session_id(&session_id)?;
    
    // CRITICAL FIX: Verify game hasn't expired
    game_session.validate_not_expired()?;
    
    // CRITICAL FIX: Additional validation checks
    require!(
        game_session.status == GameStatus::InProgress,
        WagerError::GameNotInProgress
    );
    
    // CRITICAL FIX: Validate team numbers
    require!(
        killer_team == 0 || killer_team == 1,
        WagerError::InvalidTeamSelection
    );
    
    require!(
        victim_team == 0 || victim_team == 1,
        WagerError::InvalidTeamSelection
    );
    
    // CRITICAL FIX: Ensure cross-team kill (no team kills)
    require!(
        killer_team != victim_team,
        WagerError::InvalidKillTarget
    );
    
    // CRITICAL FIX: Verify both killer and victim are actual players in specified teams
    let _killer_index = game_session.get_player_index(killer_team, killer)?;
    let _victim_index = game_session.get_player_index(victim_team, victim)?;
    
    // CRITICAL FIX: Verify game server is the actual session authority
    require!(
        ctx.accounts.game_server.key() == game_session.authority,
        WagerError::UnauthorizedKill
    );
    
    // Record the kill with all validations
    game_session.add_kill(killer_team, killer, victim_team, victim)?;
    
    msg!("Kill recorded: Player {} (team {}) killed Player {} (team {})", 
         killer, killer_team, victim, victim_team);
    
    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct RecordKill<'info> {
    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
        constraint = game_session.authority == game_server.key() @ WagerError::UnauthorizedKill,
    )]
    pub game_session: Account<'info, GameSession>,

    pub game_server: Signer<'info>,
}