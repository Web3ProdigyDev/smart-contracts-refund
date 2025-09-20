// record_kill.rs - COMPREHENSIVE SECURITY HARDENED VERSION
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
    
    // CRITICAL FIX: Enhanced validation sequence
    validate_session_id(&session_id)?;
    
    // CRITICAL FIX: Comprehensive game state validation
    game_session.validate_not_expired_safe()?;
    
    require!(
        game_session.status == GameStatus::InProgress,
        WagerError::GameNotInProgress
    );
    
    // CRITICAL FIX: Enhanced team validation
    require!(
        killer_team == 0 || killer_team == 1,
        WagerError::InvalidTeamSelection
    );
    
    require!(
        victim_team == 0 || victim_team == 1,
        WagerError::InvalidTeamSelection
    );
    
    // CRITICAL FIX: Prevent same-team kills (friendly fire prevention)
    require!(
        killer_team != victim_team,
        WagerError::InvalidKillTarget
    );
    
    // CRITICAL FIX: Validate killer and victim are different players
    require!(
        killer != victim,
        WagerError::InvalidKillTarget
    );
    
    // CRITICAL FIX: Ensure neither killer nor victim is default/empty pubkey
    require!(
        killer != Pubkey::default() && victim != Pubkey::default(),
        WagerError::InvalidPlayer
    );
    
    // CRITICAL FIX: Verify game server authority with enhanced validation
    require!(
        ctx.accounts.game_server.key() == game_session.authority,
        WagerError::UnauthorizedKill
    );
    
    require!(
        ctx.accounts.game_server.is_signer,
        WagerError::UnauthorizedKill
    );
    
    // CRITICAL FIX: Enhanced player validation - verify both players exist in their claimed teams
    let killer_index = game_session.get_player_index(killer_team, killer)?;
    let victim_index = game_session.get_player_index(victim_team, victim)?;
    
    // Additional bounds checking for array access safety
    let max_players = game_session.game_mode.players_per_team();
    require!(
        killer_index < max_players && victim_index < max_players,
        WagerError::InvalidPlayer
    );
    
    // CRITICAL FIX: Pre-validate victim has spawns available (prevents invalid state)
    let victim_spawns = match victim_team {
        0 => {
            require!(victim_index < game_session.team_a.player_spawns.len(), WagerError::InvalidPlayer);
            game_session.team_a.player_spawns[victim_index]
        },
        1 => {
            require!(victim_index < game_session.team_b.player_spawns.len(), WagerError::InvalidPlayer);
            game_session.team_b.player_spawns[victim_index]
        },
        _ => return Err(error!(WagerError::InvalidTeam)),
    };
    
    require!(
        victim_spawns > 0,
        WagerError::PlayerHasNoSpawns
    );
    
    // Pre-validate killer's kill count won't overflow
    let killer_kills = match killer_team {
        0 => {
            require!(killer_index < game_session.team_a.player_kills.len(), WagerError::InvalidPlayer);
            game_session.team_a.player_kills[killer_index]
        },
        1 => {
            require!(killer_index < game_session.team_b.player_kills.len(), WagerError::InvalidPlayer);
            game_session.team_b.player_kills[killer_index]
        },
        _ => return Err(error!(WagerError::InvalidTeam)),
    };
    
    require!(
        killer_kills < u8::MAX,
        WagerError::ArithmeticError
    );
    
    msg!("Kill validation passed: Player {} (team {}, index {}, kills: {}) killing Player {} (team {}, index {}, spawns: {})", 
         killer, killer_team, killer_index, killer_kills,
         victim, victim_team, victim_index, victim_spawns);
    
    // CRITICAL FIX: Record the kill with comprehensive state validation
    // Store original values for verification
    let original_victim_spawns = victim_spawns;
    let original_killer_kills = killer_kills;
    
    // Perform the kill recording with all internal validations
    game_session.add_kill(killer_team, killer, victim_team, victim)?;
    
    // CRITICAL FIX: Post-operation validation to ensure state was updated correctly
    let new_victim_spawns = match victim_team {
        0 => game_session.team_a.player_spawns[victim_index],
        1 => game_session.team_b.player_spawns[victim_index],
        _ => return Err(error!(WagerError::InvalidTeam)),
    };
    
    let new_killer_kills = match killer_team {
        0 => game_session.team_a.player_kills[killer_index],
        1 => game_session.team_b.player_kills[killer_index],
        _ => return Err(error!(WagerError::InvalidTeam)),
    };
    
    // Verify state changes are exactly what we expected
    require!(
        new_victim_spawns == original_victim_spawns - 1,
        WagerError::ArithmeticError
    );
    
    require!(
        new_killer_kills == original_killer_kills + 1,
        WagerError::ArithmeticError
    );
    
    msg!("Kill recorded successfully: Player {} (team {}) killed Player {} (team {})", 
         killer, killer_team, victim, victim_team);
    msg!("Updated stats - Killer kills: {} -> {}, Victim spawns: {} -> {}", 
         original_killer_kills, new_killer_kills, original_victim_spawns, new_victim_spawns);
    
    // Basic logging without borrowing conflicts
    msg!("Kill validation completed for session: {}", session_id);
    
    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct RecordKill<'info> {
    // UPDATED: Enhanced PDA with authority in seeds for collision prevention
    #[account(
        mut,
        seeds = [
            b"game_session", 
            session_id.as_bytes(),
            game_server.key().as_ref()
        ],
        bump = game_session.bump,
        constraint = game_session.authority == game_server.key() @ WagerError::UnauthorizedKill,
    )]
    pub game_session: Account<'info, GameSession>,

    #[account(
        constraint = game_server.is_signer @ WagerError::UnauthorizedKill
    )]
    pub game_server: Signer<'info>,
}