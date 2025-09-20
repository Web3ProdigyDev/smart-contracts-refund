//! State accounts for the betting program - COMPILATION FIXED VERSION
use crate::errors::WagerError;
use crate::utils::{validate_kill_count, validate_spawn_count};
use anchor_lang::prelude::*;

/// Game mode defining the team sizes
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Debug)]
pub enum GameMode {
    WinnerTakesAllOneVsOne,     // 1v1 game mode
    WinnerTakesAllThreeVsThree, // 3v3 game mode
    WinnerTakesAllFiveVsFive,   // 5v5 game mode
    PayToSpawnOneVsOne,         // 1v1 game mode
    PayToSpawnThreeVsThree,     // 3v3 game mode
    PayToSpawnFiveVsFive,       // 5v5 game mode
}

impl GameMode {
    /// Returns the required number of players per team
    pub fn players_per_team(&self) -> usize {
        match self {
            Self::WinnerTakesAllOneVsOne => 1,
            Self::WinnerTakesAllThreeVsThree => 3,
            Self::WinnerTakesAllFiveVsFive => 5,
            Self::PayToSpawnOneVsOne => 1,
            Self::PayToSpawnThreeVsThree => 3,
            Self::PayToSpawnFiveVsFive => 5,
        }
    }

    /// ENHANCED: Get maximum allowed players to prevent array bounds issues
    pub fn max_players_per_team(&self) -> usize {
        5 // Always 5 since we use fixed arrays
    }

    /// ENHANCED: Validate team size is within bounds
    pub fn validate_team_size(&self, team_size: usize) -> Result<()> {
        require!(
            team_size == self.players_per_team(),
            WagerError::InvalidTeamComposition
        );
        Ok(())
    }
}

/// Status of a game session - ENHANCED with more specific states
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum GameStatus {
    WaitingForPlayers, // Waiting for players to join
    InProgress,        // Game is active with all players joined
    Completed,         // Game has finished and rewards distributed
    Cancelled,         // Game was cancelled and refunds processed
    Expired,          // Game expired without completion
}

impl Default for GameStatus {
    fn default() -> Self {
        Self::WaitingForPlayers
    }
}

impl GameStatus {
    /// ENHANCED: Check if status allows new players to join
    pub fn allows_joining(&self) -> bool {
        matches!(self, Self::WaitingForPlayers)
    }

    /// ENHANCED: Check if status allows game operations (kills, spawns)
    pub fn allows_game_operations(&self) -> bool {
        matches!(self, Self::InProgress)
    }

    /// ENHANCED: Check if status is final (no more operations allowed)
    pub fn is_final(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Expired)
    }

    /// ENHANCED: Validate status transition is allowed
    pub fn can_transition_to(&self, new_status: &GameStatus) -> bool {
        match (self, new_status) {
            (Self::WaitingForPlayers, Self::InProgress) => true,
            (Self::WaitingForPlayers, Self::Cancelled) => true,
            (Self::InProgress, Self::Completed) => true,
            (Self::InProgress, Self::Cancelled) => true,
            (Self::WaitingForPlayers, Self::Expired) => true,
            (Self::InProgress, Self::Expired) => true,
            _ => false,
        }
    }
}

/// Represents a team in the game - COMPILATION FIXED
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Team {
    pub players: [Pubkey; 5],    // Array of player public keys
    pub total_bet: u64,          // Total amount bet by team (in lamports)  
    pub player_spawns: [u8; 5],  // u8 instead of u16 (255 max spawns)
    pub player_kills: [u8; 5],   // u8 instead of u16 (255 max kills)
}

impl Default for Team {
    fn default() -> Self {
        Self {
            players: [Pubkey::default(); 5],
            total_bet: 0,
            player_spawns: [10; 5], // Start with 10 spawns instead of 0
            player_kills: [0; 5],
        }
    }
}

impl Team {
    /// ENHANCED: Finds the first empty slot in the team with bounds checking
    pub fn get_empty_slot(&self, player_count: usize) -> Result<usize> {
        require!(
            player_count <= 5,
            WagerError::PlayerIndexOutOfBounds
        );

        self.players
            .iter()
            .enumerate()
            .take(player_count) // Only check slots we actually use
            .find(|(_, player)| **player == Pubkey::default())
            .map(|(i, _)| i)
            .ok_or_else(|| error!(WagerError::TeamIsFull))
    }
    
    /// FIXED: Check if a player is already in this team with proper bool return
    pub fn contains_player(&self, player: Pubkey, player_count: usize) -> bool {
        // COMPILATION FIX: Simple validation without require! macro
        if player_count > 5 || player == Pubkey::default() {
            return false;
        }
        
        self.players[0..player_count].iter().any(|&p| p == player)
    }

    /// ENHANCED: Validate team composition
    pub fn validate_composition(&self, expected_count: usize) -> Result<()> {
        require!(
            expected_count <= 5,
            WagerError::PlayerIndexOutOfBounds
        );

        let filled_slots = self.players[0..expected_count]
            .iter()
            .filter(|&&p| p != Pubkey::default())
            .count();

        require!(
            filled_slots == expected_count,
            WagerError::InvalidTeamComposition
        );

        // Validate no duplicates within team
        let mut seen = std::collections::HashSet::new();
        for &player in &self.players[0..expected_count] {
            if player != Pubkey::default() {
                require!(
                    seen.insert(player),
                    WagerError::DuplicatePlayer
                );
            }
        }

        Ok(())
    }

    /// ENHANCED: Get player stats safely
    pub fn get_player_stats(&self, player_index: usize) -> Result<(u8, u8)> {
        require!(
            player_index < 5,
            WagerError::PlayerIndexOutOfBounds
        );

        Ok((self.player_kills[player_index], self.player_spawns[player_index]))
    }

    /// ENHANCED: Set player stats with validation
    pub fn set_player_stats(&mut self, player_index: usize, kills: u8, spawns: u8) -> Result<()> {
        require!(
            player_index < 5,
            WagerError::PlayerIndexOutOfBounds
        );

        validate_kill_count(kills)?;
        // Don't validate spawn count here as it might be called during updates

        self.player_kills[player_index] = kills;
        self.player_spawns[player_index] = spawns;

        Ok(())
    }
}

/// Represents a game session - ENHANCED with comprehensive validation
#[account]
pub struct GameSession {
    pub session_id: String,      // 4 + variable (max 32 bytes)
    pub authority: Pubkey,       // 32 bytes
    pub session_bet: u64,        // 8 bytes
    pub game_mode: GameMode,     // 1 byte (enum)
    pub team_a: Team,            // 32*5 + 8 + 5 + 5 = 178 bytes
    pub team_b: Team,            // 32*5 + 8 + 5 + 5 = 178 bytes  
    pub status: GameStatus,      // 1 byte (enum)
    pub created_at: i64,         // 8 bytes
    pub bump: u8,                // 1 byte
    pub vault_bump: u8,          // 1 byte
    pub nonce: u64,              // ENHANCED: 8 bytes - for preventing replay attacks
    pub last_operation: i64,     // ENHANCED: 8 bytes - timestamp of last operation
}

impl GameSession {
    pub const MAX_SIZE: usize = 8 + // discriminator
        4 + 32 + // session_id (String)
        32 + // authority
        8 + // session_bet
        1 + // game_mode
        (32 * 5 + 8 + 5 + 5) + // team_a
        (32 * 5 + 8 + 5 + 5) + // team_b  
        1 + // status
        8 + // created_at
        1 + // bump
        1 + // vault_bump
        8 + // nonce
        8; // last_operation
        // Total: ~474 bytes

    /// ENHANCED: Initialize with proper validation
    pub fn initialize(
        &mut self,
        session_id: String,
        authority: Pubkey,
        session_bet: u64,
        game_mode: GameMode,
        bump: u8,
        vault_bump: u8,
    ) -> Result<()> {
        let clock = Clock::get()?;
        
        self.session_id = session_id;
        self.authority = authority;
        self.session_bet = session_bet;
        self.game_mode = game_mode;
        self.team_a = Team::default();
        self.team_b = Team::default();
        self.status = GameStatus::WaitingForPlayers;
        self.created_at = clock.unix_timestamp;
        self.bump = bump;
        self.vault_bump = vault_bump;
        self.nonce = 0;
        self.last_operation = clock.unix_timestamp;

        Ok(())
    }

    /// ENHANCED: Update last operation timestamp and increment nonce
    pub fn update_operation_tracking(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        self.last_operation = clock.unix_timestamp;
        self.nonce = self.nonce.checked_add(1).ok_or(WagerError::ArithmeticError)?;
        Ok(())
    }

    /// Gets an empty slot for a player in the specified team
    pub fn get_player_empty_slot(&self, team: u8) -> Result<usize> {
        let player_count = self.game_mode.players_per_team();
        match team {
            0 => self.team_a.get_empty_slot(player_count),
            1 => self.team_b.get_empty_slot(player_count),
            _ => Err(error!(WagerError::InvalidTeam)),
        }
    }

    /// ENHANCED: Check all teams are filled with comprehensive validation
    pub fn check_all_filled_secure(&self) -> Result<bool> {
        let player_count = self.game_mode.players_per_team();
        
        // Validate team compositions first
        self.team_a.validate_composition(player_count)?;
        self.team_b.validate_composition(player_count)?;

        // Check no player appears in both teams
        for i in 0..player_count {
            let player_a = self.team_a.players[i];
            if player_a != Pubkey::default() {
                for j in 0..player_count {
                    require!(
                        self.team_b.players[j] != player_a,
                        WagerError::DuplicatePlayer
                    );
                }
            }
        }

        Ok(true)
    }

    /// Legacy method for backward compatibility
    pub fn check_all_filled(&self) -> Result<bool> {
        self.check_all_filled_secure()
    }

    /// ENHANCED: Check if game mode supports pay-to-spawn
    pub fn is_pay_to_spawn(&self) -> bool {
        matches!(
            self.game_mode,
            GameMode::PayToSpawnOneVsOne
                | GameMode::PayToSpawnThreeVsThree
                | GameMode::PayToSpawnFiveVsFive
        )
    }

    /// ENHANCED: Get all active players with validation
    pub fn get_all_players(&self) -> Vec<Pubkey> {
        let mut players = Vec::with_capacity(10);
        let player_count = self.game_mode.players_per_team();
        
        // Only include actual players (not default pubkeys)
        for i in 0..player_count {
            let player_a = self.team_a.players[i];
            let player_b = self.team_b.players[i];
            
            if player_a != Pubkey::default() {
                players.push(player_a);
            }
            if player_b != Pubkey::default() {
                players.push(player_b);
            }
        }
        
        players
    }

    /// ENHANCED: Get player index with comprehensive validation
    pub fn get_player_index(&self, team: u8, player: Pubkey) -> Result<usize> {
        require!(team == 0 || team == 1, WagerError::InvalidTeam);
        require!(player != Pubkey::default(), WagerError::InvalidPlayer);
        
        let player_count = self.game_mode.players_per_team();
        let team_players = match team {
            0 => &self.team_a.players[0..player_count],
            1 => &self.team_b.players[0..player_count],
            _ => return Err(error!(WagerError::InvalidTeam)),
        };

        team_players
            .iter()
            .position(|p| *p == player)
            .ok_or(error!(WagerError::PlayerNotFound))
    }

    /// ENHANCED: Gets the kill and spawn sum for a player with validation
    pub fn get_kills_and_spawns(&self, player_pubkey: Pubkey) -> Result<u16> {
        require!(player_pubkey != Pubkey::default(), WagerError::InvalidPlayer);
        
        let player_count = self.game_mode.players_per_team();
        
        // Check team A
        if let Some(idx) = self.team_a.players[0..player_count]
            .iter()
            .position(|p| *p == player_pubkey) 
        {
            let kills = self.team_a.player_kills[idx] as u16;
            let spawns = self.team_a.player_spawns[idx] as u16;
            return Ok(kills.checked_add(spawns).ok_or(WagerError::ArithmeticError)?);
        }
        
        // Check team B
        if let Some(idx) = self.team_b.players[0..player_count]
            .iter()
            .position(|p| *p == player_pubkey) 
        {
            let kills = self.team_b.player_kills[idx] as u16;
            let spawns = self.team_b.player_spawns[idx] as u16;
            return Ok(kills.checked_add(spawns).ok_or(WagerError::ArithmeticError)?);
        }
        
        Err(error!(WagerError::PlayerNotFound))
    }

    /// ENHANCED: Add kill with comprehensive validation and atomic operations
    pub fn add_kill(
        &mut self,
        killer_team: u8,
        killer: Pubkey,
        victim_team: u8,
        victim: Pubkey,
    ) -> Result<()> {
        // Pre-validation checks
        require!(
            self.status.allows_game_operations(),
            WagerError::GameNotInProgress
        );

        require!(
            killer_team != victim_team,
            WagerError::InvalidKillTarget
        );

        require!(
            killer != victim && killer != Pubkey::default() && victim != Pubkey::default(),
            WagerError::InvalidKillTarget
        );

        // Get player indices with validation
        let killer_idx = self.get_player_index(killer_team, killer)?;
        let victim_idx = self.get_player_index(victim_team, victim)?;

        // ATOMIC OPERATIONS: Get current values, validate, then update
        let (victim_spawns, killer_kills) = match (killer_team, victim_team) {
            (0, 1) => {
                let victim_spawns = self.team_b.player_spawns[victim_idx];
                let killer_kills = self.team_a.player_kills[killer_idx];
                (victim_spawns, killer_kills)
            },
            (1, 0) => {
                let victim_spawns = self.team_a.player_spawns[victim_idx];
                let killer_kills = self.team_b.player_kills[killer_idx];
                (victim_spawns, killer_kills)
            },
            _ => return Err(error!(WagerError::InvalidTeam)),
        };

        // Validate victim has spawns and killer won't overflow
        require!(victim_spawns > 0, WagerError::PlayerHasNoSpawns);
        validate_kill_count(killer_kills)?;

        // Perform atomic updates
        match (killer_team, victim_team) {
            (0, 1) => {
                self.team_b.player_spawns[victim_idx] = victim_spawns - 1;
                self.team_a.player_kills[killer_idx] = killer_kills
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            },
            (1, 0) => {
                self.team_a.player_spawns[victim_idx] = victim_spawns - 1;
                self.team_b.player_kills[killer_idx] = killer_kills
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            },
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        // Update operation tracking
        self.update_operation_tracking()?;

        Ok(())
    }

    /// ENHANCED: Add spawns with comprehensive validation and consistent increment
    pub fn add_spawns_safe(&mut self, team: u8, player_index: usize) -> Result<()> {
        const SPAWN_INCREMENT: u8 = 10;
        
        require!(team == 0 || team == 1, WagerError::InvalidTeam);
        require!(player_index < self.game_mode.players_per_team(), WagerError::PlayerIndexOutOfBounds);

        let current_spawns = match team {
            0 => self.team_a.player_spawns[player_index],
            1 => self.team_b.player_spawns[player_index],
            _ => return Err(error!(WagerError::InvalidTeam)),
        };

        // Validate spawn addition using utility function
        validate_spawn_count(current_spawns, SPAWN_INCREMENT)?;

        // Perform atomic update
        match team {
            0 => {
                self.team_a.player_spawns[player_index] = current_spawns
                    .checked_add(SPAWN_INCREMENT)
                    .ok_or(WagerError::ArithmeticError)?;
            },
            1 => {
                self.team_b.player_spawns[player_index] = current_spawns
                    .checked_add(SPAWN_INCREMENT)
                    .ok_or(WagerError::ArithmeticError)?;
            },
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        // Update operation tracking
        self.update_operation_tracking()?;

        Ok(())
    }

    /// Legacy method - redirects to safe version
    pub fn add_spawns(&mut self, team: u8, player_index: usize) -> Result<()> {
        self.add_spawns_safe(team, player_index)
    }
    
    /// ENHANCED: Check if player is already joined with cross-team validation
    pub fn is_player_already_joined(&self, player: Pubkey) -> Result<bool> {
        require!(player != Pubkey::default(), WagerError::InvalidPlayer);
        
        let player_count = self.game_mode.players_per_team();
        
        Ok(self.team_a.contains_player(player, player_count) || 
           self.team_b.contains_player(player, player_count))
    }
    
    /// ENHANCED: Safe timestamp validation with comprehensive checks
    pub fn validate_not_expired_safe(&self) -> Result<()> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        
        // Validate timestamps are reasonable
        require!(
            current_time > 0 && self.created_at > 0,
            WagerError::InvalidTimestamp
        );
        
        require!(
            current_time >= self.created_at,
            WagerError::InvalidTimestamp
        );
        
        // Calculate age safely
        let game_age = current_time.saturating_sub(self.created_at);
        const GAME_TIMEOUT_SECONDS: i64 = 24 * 60 * 60; // 24 hours
        
        require!(
            game_age <= GAME_TIMEOUT_SECONDS,
            WagerError::GameTimeout
        );
        
        Ok(())
    }
    
    /// Legacy method for backward compatibility
    pub fn validate_not_expired(&self) -> Result<()> {
        self.validate_not_expired_safe()
    }
    
    /// ENHANCED: Validate and perform status transitions atomically
    pub fn transition_status(&mut self, new_status: GameStatus) -> Result<()> {
        require!(
            self.status.can_transition_to(&new_status),
            WagerError::InvalidGameState
        );

        match (&self.status, &new_status) {
            (GameStatus::WaitingForPlayers, GameStatus::InProgress) => {
                require!(
                    self.check_all_filled_secure()?,
                    WagerError::NotAllPlayersJoined
                );
            },
            (GameStatus::InProgress, GameStatus::Completed) => {
                // Valid transition
            },
            (_, GameStatus::Cancelled) => {
                // Can cancel from most states
            },
            (_, GameStatus::Expired) => {
                // Can expire from non-final states
                require!(
                    !self.status.is_final(),
                    WagerError::InvalidGameState
                );
            },
            _ => return Err(error!(WagerError::InvalidGameState)),
        }

        self.status = new_status;
        self.update_operation_tracking()?;
        
        Ok(())
    }

    /// ENHANCED: Comprehensive game session validation
    pub fn validate_integrity(&self) -> Result<()> {
        // Validate basic fields
        require!(
            !self.session_id.is_empty() && self.session_id.len() <= 32,
            WagerError::InvalidSessionId
        );

        require!(
            self.authority != Pubkey::default(),
            WagerError::AuthorityMismatch
        );

        require!(
            self.session_bet > 0,
            WagerError::InvalidBetAmount
        );

        // Validate timestamps
        require!(
            self.created_at > 0 && self.last_operation >= self.created_at,
            WagerError::InvalidTimestamp
        );

        // Validate team compositions if game is in progress or completed
        if matches!(self.status, GameStatus::InProgress | GameStatus::Completed) {
            let player_count = self.game_mode.players_per_team();
            self.team_a.validate_composition(player_count)?;
            self.team_b.validate_composition(player_count)?;
        }

        Ok(())
    }

    /// ENHANCED: Get comprehensive game statistics
    pub fn get_game_stats(&self) -> Result<GameStats> {
        let player_count = self.game_mode.players_per_team();
        
        let mut team_a_kills = 0u16;
        let mut team_a_spawns = 0u16;
        let mut team_b_kills = 0u16;
        let mut team_b_spawns = 0u16;

        for i in 0..player_count {
            team_a_kills = team_a_kills
                .checked_add(self.team_a.player_kills[i] as u16)
                .ok_or(WagerError::ArithmeticError)?;
            team_a_spawns = team_a_spawns
                .checked_add(self.team_a.player_spawns[i] as u16)
                .ok_or(WagerError::ArithmeticError)?;
            team_b_kills = team_b_kills
                .checked_add(self.team_b.player_kills[i] as u16)
                .ok_or(WagerError::ArithmeticError)?;
            team_b_spawns = team_b_spawns
                .checked_add(self.team_b.player_spawns[i] as u16)
                .ok_or(WagerError::ArithmeticError)?;
        }

        Ok(GameStats {
            team_a_kills,
            team_a_spawns,
            team_b_kills,
            team_b_spawns,
            total_pot: self.team_a.total_bet
                .checked_add(self.team_b.total_bet)
                .ok_or(WagerError::ArithmeticError)?,
            active_players: self.get_all_players().len() as u8,
        })
    }
}

/// ENHANCED: Game statistics structure
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct GameStats {
    pub team_a_kills: u16,
    pub team_a_spawns: u16,
    pub team_b_kills: u16,
    pub team_b_spawns: u16,
    pub total_pot: u64,
    pub active_players: u8,
}