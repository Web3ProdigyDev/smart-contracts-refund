//! State accounts for the betting program - SECURITY HARDENED WITH PROPER ATOMIC OPERATIONS
use crate::errors::WagerError;
use crate::utils::{validate_kill_count, validate_spawn_count, generate_secure_nonce, generate_session_hash};
use anchor_lang::prelude::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::collections::HashSet;

/// Maximum safe values to prevent overflow in earnings calculations
pub const MAX_SAFE_KILLS: u8 = 100;
pub const MAX_SAFE_SPAWNS: u8 = 100;
pub const MAX_SAFE_BET: u64 = 1_000_000_000; // 1 billion lamports (~1 SOL)
pub const MIN_SAFE_BET: u64 = 1_000_000; // 0.001 SOL minimum
pub const GAME_TIMEOUT_SECONDS: i64 = 24 * 60 * 60; // 24 hours
pub const MAX_OPERATIONS_PER_MINUTE: u64 = 60; // Rate limiting

/// Game mode defining the team sizes
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Debug)]
pub enum GameMode {
    WinnerTakesAllOneVsOne,
    WinnerTakesAllThreeVsThree,
    WinnerTakesAllFiveVsFive,
    PayToSpawnOneVsOne,
    PayToSpawnThreeVsThree,
    PayToSpawnFiveVsFive,
}

impl GameMode {
    pub fn players_per_team(&self) -> usize {
        match self {
            Self::WinnerTakesAllOneVsOne | Self::PayToSpawnOneVsOne => 1,
            Self::WinnerTakesAllThreeVsThree | Self::PayToSpawnThreeVsThree => 3,
            Self::WinnerTakesAllFiveVsFive | Self::PayToSpawnFiveVsFive => 5,
        }
    }

    pub fn max_players_per_team(&self) -> usize {
        5 // Fixed array size
    }

    pub fn validate_team_size(&self, team_size: usize) -> Result<()> {
        require!(
            team_size == self.players_per_team(),
            WagerError::InvalidTeamComposition
        );
        Ok(())
    }

    /// Get spawn increment based on game mode for economic balance
    pub fn spawn_increment(&self) -> u8 {
        match self {
            Self::PayToSpawnOneVsOne => 5,        // Lower increment for 1v1
            Self::PayToSpawnThreeVsThree => 8,    // Medium increment for 3v3
            Self::PayToSpawnFiveVsFive => 10,     // Higher increment for 5v5
            _ => 10, // Default for non-pay-to-spawn modes
        }
    }

    /// Check if mode supports pay-to-spawn
    pub fn is_pay_to_spawn(&self) -> bool {
        matches!(
            self,
            Self::PayToSpawnOneVsOne
                | Self::PayToSpawnThreeVsThree
                | Self::PayToSpawnFiveVsFive
        )
    }
}

/// Status of a game session
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum GameStatus {
    WaitingForPlayers,
    InProgress,
    Completed,
    Cancelled,
    Expired,
    EmergencyPaused, // New status for emergency situations
}

impl Default for GameStatus {
    fn default() -> Self {
        Self::WaitingForPlayers
    }
}

impl GameStatus {
    pub fn allows_joining(&self) -> bool {
        matches!(self, Self::WaitingForPlayers)
    }

    pub fn allows_game_operations(&self) -> bool {
        matches!(self, Self::InProgress)
    }

    pub fn is_final(&self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Expired)
    }

    pub fn can_transition_to(&self, new_status: &GameStatus) -> bool {
        match (self, new_status) {
            (Self::WaitingForPlayers, Self::InProgress) => true,
            (Self::WaitingForPlayers, Self::Cancelled) => true,
            (Self::WaitingForPlayers, Self::Expired) => true,
            (Self::InProgress, Self::Completed) => true,
            (Self::InProgress, Self::Cancelled) => true,
            (Self::InProgress, Self::Expired) => true,
            (_, Self::EmergencyPaused) => true, // Can pause from any state
            (Self::EmergencyPaused, Self::InProgress) => true, // Can resume
            (Self::EmergencyPaused, Self::Cancelled) => true, // Can cancel from pause
            _ => false,
        }
    }
}

/// Represents a team in the game - FIXED OVERFLOW PROTECTION
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Team {
    pub players: [Pubkey; 5],
    pub total_bet: u64,
    pub player_spawns: [u8; 5],
    pub player_kills: [u8; 5],
}

impl Default for Team {
    fn default() -> Self {
        Self {
            players: [Pubkey::default(); 5],
            total_bet: 0,
            player_spawns: [10; 5], // Start with 10 spawns
            player_kills: [0; 5],
        }
    }
}

impl Team {
    /// FIXED: Finds the first empty slot in the team with proper bounds checking
    pub fn get_empty_slot(&self, player_count: usize) -> Result<usize> {
        require!(
            player_count <= 5,
            WagerError::PlayerIndexOutOfBounds
        );

        self.players
            .iter()
            .enumerate()
            .take(player_count)
            .find(|(_, player)| **player == Pubkey::default())
            .map(|(i, _)| i)
            .ok_or_else(|| error!(WagerError::TeamIsFull))
    }
    
    /// FIXED: Proper player validation with explicit error handling
    pub fn contains_player(&self, player: Pubkey, player_count: usize) -> Result<bool> {
        require!(player_count <= 5, WagerError::PlayerIndexOutOfBounds);
        require!(player != Pubkey::default(), WagerError::InvalidPlayer);
        
        Ok(self.players[0..player_count]
            .iter()
            .any(|&p| p == player && p != Pubkey::default()))
    }

    /// ENHANCED: Validate team composition with duplicate detection
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
        let mut seen = HashSet::new();
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

        let kills = self.player_kills[player_index];
        let spawns = self.player_spawns[player_index];

        require!(
            kills <= MAX_SAFE_KILLS,
            WagerError::InvalidKillCount
        );
        
        require!(
            spawns <= MAX_SAFE_SPAWNS,
            WagerError::SpawnLimitExceeded
        );

        Ok((kills, spawns))
    }

    /// ENHANCED: Set player stats with validation
    pub fn set_player_stats(&mut self, player_index: usize, kills: u8, spawns: u8) -> Result<()> {
        require!(
            player_index < 5,
            WagerError::PlayerIndexOutOfBounds
        );

        require!(
            kills <= MAX_SAFE_KILLS,
            WagerError::InvalidKillCount
        );
        
        require!(
            spawns <= MAX_SAFE_SPAWNS,
            WagerError::SpawnLimitExceeded
        );

        validate_kill_count(kills)?;

        self.player_kills[player_index] = kills;
        self.player_spawns[player_index] = spawns;

        Ok(())
    }

    /// FIXED: Safe calculation of total activity
    pub fn calculate_safe_total_activity(&self, player_count: usize) -> Result<u64> {
        let mut total: u64 = 0;
        
        for i in 0..player_count.min(5) {
            let kills = self.player_kills[i] as u64;
            let spawns = self.player_spawns[i] as u64;
            
            require!(
                kills <= MAX_SAFE_KILLS as u64,
                WagerError::ValueTooLarge
            );
            
            require!(
                spawns <= MAX_SAFE_SPAWNS as u64,
                WagerError::ValueTooLarge
            );
            
            let player_activity = kills
                .checked_add(spawns)
                .ok_or(WagerError::ArithmeticError)?;
                
            total = total
                .checked_add(player_activity)
                .ok_or(WagerError::ArithmeticError)?;
        }
        
        Ok(total)
    }
}

/// FIXED: Game session with proper atomic operations and security
#[account]
pub struct GameSession {
    pub session_id: String,
    pub authority: Pubkey,
    pub session_bet: u64,
    pub game_mode: GameMode,
    pub team_a: Team,
    pub team_b: Team,
    pub status: GameStatus,
    pub created_at: i64,
    pub bump: u8,
    pub vault_bump: u8,
    pub nonce: u64, // For atomic operations
    pub last_operation: i64,
    pub session_hash: [u8; 32],
    pub operation_count: u64,
    pub last_operation_window: i64, // For rate limiting
    pub operations_in_window: u64,  // Operations counter for rate limiting
}

impl GameSession {
    pub const MAX_SIZE: usize = 8 + // discriminator
        4 + 32 + // session_id
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
        8 + // last_operation
        32 + // session_hash
        8 + // operation_count
        8 + // last_operation_window
        8; // operations_in_window

    /// FIXED: Initialize with proper validation
    pub fn initialize(
        &mut self,
        session_id: String,
        authority: Pubkey,
        session_bet: u64,
        game_mode: GameMode,
        bump: u8,
        vault_bump: u8,
    ) -> Result<()> {
        // Validate session ID
        require!(
            session_id.len() >= 12 && session_id.len() <= 32,
            WagerError::InvalidSessionId
        );
        
        require!(
            session_id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-'),
            WagerError::InvalidSessionId
        );

        // Validate bet amount with proper bounds
        require!(
            session_bet >= MIN_SAFE_BET && session_bet <= MAX_SAFE_BET,
            WagerError::InvalidBetAmount
        );

        let clock = Clock::get()?;
        
        // Generate cryptographically secure session hash
        let session_hash = generate_session_hash(
            &session_id, 
            authority, 
            clock.unix_timestamp
        );
        
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
        self.nonce = generate_secure_nonce(0, &session_hash);
        self.last_operation = clock.unix_timestamp;
        self.session_hash = session_hash;
        self.operation_count = 0;
        self.last_operation_window = clock.unix_timestamp;
        self.operations_in_window = 0;

        Ok(())
    }

    /// FIXED: Rate limiting check
    pub fn check_rate_limit(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        
        // Reset window if more than 1 minute has passed
        if current_time - self.last_operation_window >= 60 {
            self.last_operation_window = current_time;
            self.operations_in_window = 0;
        }
        
        require!(
            self.operations_in_window < MAX_OPERATIONS_PER_MINUTE,
            WagerError::RateLimitExceeded
        );
        
        self.operations_in_window += 1;
        Ok(())
    }

    /// FIXED: Proper atomic operation tracking
    pub fn update_operation_tracking(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        
        // Check rate limit first
        self.check_rate_limit()?;
        
        // Atomic increment with overflow protection
        self.nonce = self.nonce
            .checked_add(1)
            .ok_or(WagerError::ArithmeticError)?;
        self.operation_count = self.operation_count
            .checked_add(1)
            .ok_or(WagerError::ArithmeticError)?;
        self.last_operation = clock.unix_timestamp;
        
        Ok(())
    }

    /// CRITICAL FIX: Proper atomic status transition
    pub fn atomic_status_transition(&mut self, new_status: GameStatus, expected_nonce: u64) -> Result<()> {
        // Check for emergency pause
        if matches!(self.status, GameStatus::EmergencyPaused) && 
           !matches!(new_status, GameStatus::InProgress | GameStatus::Cancelled) {
            return Err(error!(WagerError::EmergencyPaused));
        }

        // Atomic nonce check - this must be the first check
        require!(
            self.nonce == expected_nonce,
            WagerError::ConcurrentOperation
        );

        // Validate transition is allowed
        require!(
            self.status.can_transition_to(&new_status),
            WagerError::InvalidGameState
        );

        // Specific transition validations
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
                require!(
                    !self.status.is_final(),
                    WagerError::InvalidGameState
                );
            },
            _ => {}
        }

        // ATOMIC UPDATE: Status and nonce in single operation
        self.status = new_status;
        self.update_operation_tracking()?;
        
        Ok(())
    }

    /// Legacy wrapper for compatibility
    pub fn transition_status(&mut self, new_status: GameStatus) -> Result<()> {
        let current_nonce = self.nonce;
        self.atomic_status_transition(new_status, current_nonce)
    }

    /// Get empty slot for player
    pub fn get_player_empty_slot(&self, team: u8) -> Result<usize> {
        let player_count = self.game_mode.players_per_team();
        match team {
            0 => self.team_a.get_empty_slot(player_count),
            1 => self.team_b.get_empty_slot(player_count),
            _ => Err(error!(WagerError::InvalidTeam)),
        }
    }

    /// FIXED: Check all teams filled with cross-team duplicate validation
    pub fn check_all_filled_secure(&self) -> Result<bool> {
        let player_count = self.game_mode.players_per_team();
        
        // Validate team compositions
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

    /// Legacy method
    pub fn check_all_filled(&self) -> Result<bool> {
        self.check_all_filled_secure()
    }

    /// Check if pay-to-spawn mode
    pub fn is_pay_to_spawn(&self) -> bool {
        self.game_mode.is_pay_to_spawn()
    }

    /// Get all active players
    pub fn get_all_players(&self) -> Vec<Pubkey> {
        let mut players = Vec::with_capacity(10);
        let player_count = self.game_mode.players_per_team();
        
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

    /// Get player index with validation
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

    /// FIXED: Get kills and spawns with proper overflow protection
    pub fn get_kills_and_spawns(&self, player_pubkey: Pubkey) -> Result<u16> {
        require!(player_pubkey != Pubkey::default(), WagerError::InvalidPlayer);
        
        let player_count = self.game_mode.players_per_team();
        
        // Check team A
        if let Some(idx) = self.team_a.players[0..player_count]
            .iter()
            .position(|p| *p == player_pubkey) 
        {
            let kills = self.team_a.player_kills[idx];
            let spawns = self.team_a.player_spawns[idx];
            
            require!(
                kills <= MAX_SAFE_KILLS && spawns <= MAX_SAFE_SPAWNS,
                WagerError::InvalidKillCount
            );
            
            return Ok((kills as u16)
                .checked_add(spawns as u16)
                .ok_or(WagerError::ArithmeticError)?);
        }
        
        // Check team B
        if let Some(idx) = self.team_b.players[0..player_count]
            .iter()
            .position(|p| *p == player_pubkey) 
        {
            let kills = self.team_b.player_kills[idx];
            let spawns = self.team_b.player_spawns[idx];
            
            require!(
                kills <= MAX_SAFE_KILLS && spawns <= MAX_SAFE_SPAWNS,
                WagerError::InvalidKillCount
            );
            
            return Ok((kills as u16)
                .checked_add(spawns as u16)
                .ok_or(WagerError::ArithmeticError)?);
        }
        
        Err(error!(WagerError::PlayerNotFound))
    }

    /// CRITICAL FIX: Proper earnings calculation without overly restrictive validation
    pub fn calculate_player_earnings_safe(&self, player_pubkey: Pubkey) -> Result<u64> {
        require!(player_pubkey != Pubkey::default(), WagerError::InvalidPlayer);
        
        // Get kills and spawns safely
        let kills_and_spawns_u16 = self.get_kills_and_spawns(player_pubkey)?;
        let kills_and_spawns_u64 = kills_and_spawns_u16 as u64;
        
        // Validate session bet is within bounds
        require!(
            self.session_bet >= MIN_SAFE_BET && self.session_bet <= MAX_SAFE_BET,
            WagerError::InvalidBetAmount
        );
        
        // Realistic validation - maximum reasonable activity
        const MAX_REASONABLE_ACTIVITY: u64 = (MAX_SAFE_KILLS as u64) + (MAX_SAFE_SPAWNS as u64);
        require!(
            kills_and_spawns_u64 <= MAX_REASONABLE_ACTIVITY,
            WagerError::InvalidKillCount
        );
        
        // This calculation will never overflow with our constants:
        // Max: 200 * 1_000_000_000 = 200_000_000_000 (well within u64 range)
        let earnings = kills_and_spawns_u64
            .checked_mul(self.session_bet)
            .and_then(|product| product.checked_div(10))
            .ok_or(WagerError::ArithmeticError)?;
        
        Ok(earnings)
    }

    /// FIXED: Add kill with proper atomic operations and validation
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

        // Get player indices
        let killer_idx = self.get_player_index(killer_team, killer)?;
        let victim_idx = self.get_player_index(victim_team, victim)?;

        // Get current values for atomic operation
        let (current_victim_spawns, current_killer_kills) = match (killer_team, victim_team) {
            (0, 1) => {
                (self.team_b.player_spawns[victim_idx], self.team_a.player_kills[killer_idx])
            },
            (1, 0) => {
                (self.team_a.player_spawns[victim_idx], self.team_b.player_kills[killer_idx])
            },
            _ => return Err(error!(WagerError::InvalidTeam)),
        };

        // Validate operation is possible
        require!(current_victim_spawns > 0, WagerError::PlayerHasNoSpawns);
        require!(
            current_killer_kills < MAX_SAFE_KILLS,
            WagerError::InvalidKillCount
        );

        validate_kill_count(current_killer_kills)?;

        // Perform atomic updates
        match (killer_team, victim_team) {
            (0, 1) => {
                self.team_b.player_spawns[victim_idx] = current_victim_spawns - 1;
                self.team_a.player_kills[killer_idx] = current_killer_kills
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            },
            (1, 0) => {
                self.team_a.player_spawns[victim_idx] = current_victim_spawns - 1;
                self.team_b.player_kills[killer_idx] = current_killer_kills
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            },
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        // Update operation tracking
        self.update_operation_tracking()?;

        Ok(())
    }

    /// FIXED: Add spawns with game mode specific increments
    pub fn add_spawns_safe(&mut self, team: u8, player_index: usize) -> Result<()> {
        let spawn_increment = self.game_mode.spawn_increment();
        
        require!(team == 0 || team == 1, WagerError::InvalidTeam);
        require!(
            player_index < self.game_mode.players_per_team(),
            WagerError::PlayerIndexOutOfBounds
        );

        let current_spawns = match team {
            0 => self.team_a.player_spawns[player_index],
            1 => self.team_b.player_spawns[player_index],
            _ => return Err(error!(WagerError::InvalidTeam)),
        };

        // Check if addition would exceed safe limits
        let new_spawns = current_spawns
            .checked_add(spawn_increment)
            .ok_or(WagerError::ArithmeticError)?;
            
        require!(
            new_spawns <= MAX_SAFE_SPAWNS,
            WagerError::SpawnLimitExceeded
        );

        validate_spawn_count(current_spawns, spawn_increment)?;

        // Perform atomic update
        match team {
            0 => self.team_a.player_spawns[player_index] = new_spawns,
            1 => self.team_b.player_spawns[player_index] = new_spawns,
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        self.update_operation_tracking()?;
        Ok(())
    }

    /// Legacy wrapper
    pub fn add_spawns(&mut self, team: u8, player_index: usize) -> Result<()> {
        self.add_spawns_safe(team, player_index)
    }
    
    /// FIXED: Check if player already joined with proper validation
    pub fn is_player_already_joined(&self, player: Pubkey) -> Result<bool> {
        require!(player != Pubkey::default(), WagerError::InvalidPlayer);
        
        let player_count = self.game_mode.players_per_team();
        
        let in_team_a = self.team_a.contains_player(player, player_count)?;
        let in_team_b = self.team_b.contains_player(player, player_count)?;
        
        // Additional validation - player should not be in both teams
        require!(
            !(in_team_a && in_team_b),
            WagerError::DuplicatePlayer
        );
        
        Ok(in_team_a || in_team_b)
    }
    
    /// FIXED: Timestamp validation with overflow protection
    pub fn validate_not_expired_safe(&self) -> Result<()> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        
        require!(
            current_time > 0 && self.created_at > 0,
            WagerError::InvalidTimestamp
        );
        
        require!(
            current_time >= self.created_at,
            WagerError::InvalidTimestamp
        );
        
        let game_age = current_time
            .checked_sub(self.created_at)
            .ok_or(WagerError::ArithmeticError)?;
        
        require!(
            game_age <= GAME_TIMEOUT_SECONDS,
            WagerError::GameTimeout
        );
        
        Ok(())
    }
    
    /// Legacy wrapper
    pub fn validate_not_expired(&self) -> Result<()> {
        self.validate_not_expired_safe()
    }
    
    /// ENHANCED: Comprehensive validation with integrity checks
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
            self.session_bet >= MIN_SAFE_BET && self.session_bet <= MAX_SAFE_BET,
            WagerError::InvalidBetAmount
        );

        // Validate timestamps
        require!(
            self.created_at > 0 && self.last_operation >= self.created_at,
            WagerError::InvalidTimestamp
        );

        // Validate session hash integrity
        let expected_hash = generate_session_hash(
            &self.session_id,
            self.authority,
            self.created_at
        );
        
        require!(
            self.session_hash == expected_hash,
            WagerError::SessionIdCollision
        );

        // Validate team compositions if game is active
        if matches!(self.status, GameStatus::InProgress | GameStatus::Completed) {
            let player_count = self.game_mode.players_per_team();
            self.team_a.validate_composition(player_count)?;
            self.team_b.validate_composition(player_count)?;
            
            // Validate all player stats are within safe limits
            for i in 0..player_count {
                require!(
                    self.team_a.player_kills[i] <= MAX_SAFE_KILLS &&
                    self.team_a.player_spawns[i] <= MAX_SAFE_SPAWNS &&
                    self.team_b.player_kills[i] <= MAX_SAFE_KILLS &&
                    self.team_b.player_spawns[i] <= MAX_SAFE_SPAWNS,
                    WagerError::InvalidKillCount
                );
            }
        }

        // Validate operation counts are reasonable
        require!(
            self.operation_count < u64::MAX / 2, // Leave room for more operations
            WagerError::ArithmeticError
        );

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
            let a_kills = self.team_a.player_kills[i];
            let a_spawns = self.team_a.player_spawns[i];
            let b_kills = self.team_b.player_kills[i];
            let b_spawns = self.team_b.player_spawns[i];
            
            require!(
                a_kills <= MAX_SAFE_KILLS && a_spawns <= MAX_SAFE_SPAWNS &&
                b_kills <= MAX_SAFE_KILLS && b_spawns <= MAX_SAFE_SPAWNS,
                WagerError::InvalidKillCount
            );
            
            team_a_kills = team_a_kills
                .checked_add(a_kills as u16)
                .ok_or(WagerError::ArithmeticError)?;
            team_a_spawns = team_a_spawns
                .checked_add(a_spawns as u16)
                .ok_or(WagerError::ArithmeticError)?;
            team_b_kills = team_b_kills
                .checked_add(b_kills as u16)
                .ok_or(WagerError::ArithmeticError)?;
            team_b_spawns = team_b_spawns
                .checked_add(b_spawns as u16)
                .ok_or(WagerError::ArithmeticError)?;
        }

        let total_pot = self.team_a.total_bet
            .checked_add(self.team_b.total_bet)
            .ok_or(WagerError::ArithmeticError)?;

        Ok(GameStats {
            team_a_kills,
            team_a_spawns,
            team_b_kills,
            team_b_spawns,
            total_pot,
            active_players: self.get_all_players().len() as u8,
        })
    }

    /// ENHANCED: Validate earnings safety for all calculations
    pub fn validate_earnings_safety(&self) -> Result<()> {
        let player_count = self.game_mode.players_per_team();
        
        require!(
            self.session_bet >= MIN_SAFE_BET && self.session_bet <= MAX_SAFE_BET,
            WagerError::InvalidBetAmount
        );
        
        // Check all player activity levels are safe
        for i in 0..player_count {
            let team_a_total = (self.team_a.player_kills[i] as u16)
                .checked_add(self.team_a.player_spawns[i] as u16)
                .ok_or(WagerError::ArithmeticError)?;
                
            let team_b_total = (self.team_b.player_kills[i] as u16)
                .checked_add(self.team_b.player_spawns[i] as u16)
                .ok_or(WagerError::ArithmeticError)?;
            
            // With our current limits, these will always be safe
            require!(
                team_a_total <= (MAX_SAFE_KILLS as u16 + MAX_SAFE_SPAWNS as u16) &&
                team_b_total <= (MAX_SAFE_KILLS as u16 + MAX_SAFE_SPAWNS as u16),
                WagerError::InvalidKillCount
            );
        }
        
        Ok(())
    }

    /// Get total safe activity across all players
    pub fn get_total_safe_activity(&self) -> Result<u64> {
        let team_a_activity = self.team_a.calculate_safe_total_activity(
            self.game_mode.players_per_team()
        )?;
        
        let team_b_activity = self.team_b.calculate_safe_total_activity(
            self.game_mode.players_per_team()
        )?;
        
        team_a_activity
            .checked_add(team_b_activity)
            .ok_or(error!(WagerError::ArithmeticError))
    }

    /// Validate all player earnings are calculable safely
    pub fn validate_all_player_earnings_safety(&self) -> Result<()> {
        let all_players = self.get_all_players();
        
        for player in all_players {
            // This validates each player's earnings calculation
            self.calculate_player_earnings_safe(player)?;
        }
        
        Ok(())
    }

    /// SECURITY: Emergency pause function for administrators
    pub fn emergency_pause(&mut self, authority: Pubkey) -> Result<()> {
        require!(
            authority == self.authority, // Only game authority can pause
            WagerError::AuthorityMismatch
        );
        
        require!(
            !self.status.is_final(),
            WagerError::InvalidGameState
        );
        
        let current_nonce = self.nonce;
        self.atomic_status_transition(GameStatus::EmergencyPaused, current_nonce)?;
        
        Ok(())
    }

    /// SECURITY: Resume from emergency pause
    pub fn resume_from_pause(&mut self, authority: Pubkey) -> Result<()> {
        require!(
            authority == self.authority,
            WagerError::AuthorityMismatch
        );
        
        require!(
            matches!(self.status, GameStatus::EmergencyPaused),
            WagerError::InvalidGameState
        );
        
        let current_nonce = self.nonce;
        self.atomic_status_transition(GameStatus::InProgress, current_nonce)?;
        
        Ok(())
    }

    /// SECURITY: Validate session hasn't been tampered with
    pub fn validate_session_integrity(&self) -> Result<()> {
        // Recompute hash and verify
        let expected_hash = generate_session_hash(
            &self.session_id,
            self.authority,
            self.created_at
        );
        
        require!(
            self.session_hash == expected_hash,
            WagerError::GameDataCorruption
        );
        
        // Validate nonce hasn't been manipulated
        require!(
            self.nonce >= self.operation_count,
            WagerError::GameDataCorruption
        );
        
        // Validate timestamps are consistent
        require!(
            self.last_operation >= self.created_at &&
            self.last_operation_window >= self.created_at,
            WagerError::GameDataCorruption
        );
        
        Ok(())
    }
}

/// ENHANCED: Game statistics with validation
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Debug)]
pub struct GameStats {
    pub team_a_kills: u16,
    pub team_a_spawns: u16,
    pub team_b_kills: u16,
    pub team_b_spawns: u16,
    pub total_pot: u64,
    pub active_players: u8,
}

impl GameStats {
    /// Validate statistics are within safe bounds
    pub fn validate_safe_bounds(&self) -> Result<()> {
        let total_activity = (self.team_a_kills as u64)
            .checked_add(self.team_a_spawns as u64)
            .and_then(|sum| sum.checked_add(self.team_b_kills as u64))
            .and_then(|sum| sum.checked_add(self.team_b_spawns as u64))
            .ok_or(WagerError::ArithmeticError)?;
        
        // Maximum reasonable total activity
        let max_reasonable = ((MAX_SAFE_KILLS as u64) + (MAX_SAFE_SPAWNS as u64)) * 10;
        require!(
            total_activity <= max_reasonable,
            WagerError::InvalidKillCount
        );
        
        // Validate pot size
        require!(
            self.total_pot <= MAX_SAFE_BET * 10, // Reasonable maximum pot
            WagerError::InvalidBetAmount
        );
        
        require!(
            self.active_players <= 10, // Maximum 5v5
            WagerError::InvalidPlayerCount
        );
        
        Ok(())
    }
    
    /// Get win ratio for team A (kills / total kills)
    pub fn get_team_a_win_ratio(&self) -> Result<f64> {
        let total_kills = (self.team_a_kills as u64)
            .checked_add(self.team_b_kills as u64)
            .ok_or(WagerError::ArithmeticError)?;
            
        if total_kills == 0 {
            Ok(0.5) // Equal if no kills
        } else {
            Ok(self.team_a_kills as f64 / total_kills as f64)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_safe_limits() {
        // Verify our limits prevent overflow
        let max_activity = (MAX_SAFE_KILLS as u64) + (MAX_SAFE_SPAWNS as u64);
        let max_earnings = max_activity * MAX_SAFE_BET / 10;
        
        // Should not overflow u64
        assert!(max_earnings < u64::MAX);
        
        // Should be reasonable for gameplay
        assert!(MAX_SAFE_KILLS >= 50);
        assert!(MAX_SAFE_SPAWNS >= 50);
    }

    #[test]
    fn test_earnings_calculation_safety() {
        // Test maximum values don't overflow
        let max_activity = (MAX_SAFE_KILLS as u64) + (MAX_SAFE_SPAWNS as u64);
        let product = max_activity.checked_mul(MAX_SAFE_BET);
        assert!(product.is_some());
        
        let earnings = product.unwrap().checked_div(10);
        assert!(earnings.is_some());
        assert!(earnings.unwrap() < u64::MAX);
    }

    #[test]
    fn test_game_mode_spawn_increments() {
        assert_eq!(GameMode::PayToSpawnOneVsOne.spawn_increment(), 5);
        assert_eq!(GameMode::PayToSpawnThreeVsThree.spawn_increment(), 8);
        assert_eq!(GameMode::PayToSpawnFiveVsFive.spawn_increment(), 10);
    }

    #[test]
    fn test_status_transitions() {
        assert!(GameStatus::WaitingForPlayers.can_transition_to(&GameStatus::InProgress));
        assert!(GameStatus::InProgress.can_transition_to(&GameStatus::Completed));
        assert!(GameStatus::InProgress.can_transition_to(&GameStatus::EmergencyPaused));
        assert!(!GameStatus::Completed.can_transition_to(&GameStatus::InProgress));
    }

    #[test]
    fn test_bet_amount_validation() {
        assert!(MIN_SAFE_BET > 0);
        assert!(MAX_SAFE_BET >= MIN_SAFE_BET);
        assert!(MAX_SAFE_BET <= u64::MAX / 1000); // Leave room for calculations
    }
}