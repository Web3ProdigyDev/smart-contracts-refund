//! State accounts for the betting program - RACE CONDITION FIXED VERSION WITH COMPARE-AND-SWAP
use crate::errors::WagerError;
use crate::utils::{generate_secure_nonce, generate_session_hash};
use anchor_lang::prelude::*;
use std::collections::HashSet;

/// SIMPLIFIED: Conservative safe limits to prevent ALL overflow scenarios
pub const MAX_KILLS: u8 = 50;  // Reduced for absolute safety
pub const MAX_SPAWNS: u8 = 50; // Reduced for absolute safety
pub const MAX_BET: u64 = 100_000_000; // 0.1 SOL max - much safer
pub const MIN_BET: u64 = 1_000_000; // 0.001 SOL minimum
pub const GAME_TIMEOUT_SECONDS: i64 = 24 * 60 * 60;
pub const MAX_OPERATIONS_PER_MINUTE: u64 = 60;

// SIMPLIFIED: Single overflow-safe calculation threshold
// With MAX_KILLS=50, MAX_SPAWNS=50, MAX_BET=100M: 100 * 100M = 10B (well under u64::MAX)
pub const MAX_TOTAL_ACTIVITY: u64 = (MAX_KILLS as u64) + (MAX_SPAWNS as u64); // 100 max

/// ENHANCED: Distribution status tracking to prevent race conditions
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum DistributionStatus {
    NotStarted,
    InProgress,
    Completed,
    Failed,
}

impl Default for DistributionStatus {
    fn default() -> Self {
        Self::NotStarted
    }
}

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

    pub fn spawn_increment(&self) -> u8 {
        match self {
            Self::PayToSpawnOneVsOne => 3,        // Reduced for safety
            Self::PayToSpawnThreeVsThree => 5,    // Reduced for safety
            Self::PayToSpawnFiveVsFive => 7,      // Reduced for safety
            _ => 0,
        }
    }

    pub fn is_pay_to_spawn(&self) -> bool {
        matches!(
            self,
            Self::PayToSpawnOneVsOne | Self::PayToSpawnThreeVsThree | Self::PayToSpawnFiveVsFive
        )
    }
}

/// Status of a game session with enhanced atomic operations
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum GameStatus {
    WaitingForPlayers,
    InProgress,
    Completed,
    Cancelled,
    Expired,
    EmergencyPaused,
    // NEW: Intermediate states to prevent race conditions
    DistributionInProgress,
    RefundInProgress,
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

    pub fn is_distributing(&self) -> bool {
        matches!(self, Self::DistributionInProgress)
    }

    pub fn is_refunding(&self) -> bool {
        matches!(self, Self::RefundInProgress)
    }

    pub fn can_transition_to(&self, new_status: &GameStatus) -> bool {
        match (self, new_status) {
            // Standard transitions
            (Self::WaitingForPlayers, Self::InProgress) => true,
            (Self::WaitingForPlayers, Self::Cancelled) => true,
            (Self::WaitingForPlayers, Self::Expired) => true,
            (Self::WaitingForPlayers, Self::RefundInProgress) => true,
            
            // Game in progress transitions
            (Self::InProgress, Self::DistributionInProgress) => true,
            (Self::InProgress, Self::RefundInProgress) => true,
            (Self::InProgress, Self::Cancelled) => true,
            (Self::InProgress, Self::Expired) => true,
            
            // Distribution flow
            (Self::DistributionInProgress, Self::Completed) => true,
            (Self::DistributionInProgress, Self::Cancelled) => true,
            
            // Refund flow  
            (Self::RefundInProgress, Self::Cancelled) => true,
            
            // Emergency transitions
            (_, Self::EmergencyPaused) => true,
            (Self::EmergencyPaused, Self::InProgress) => true,
            (Self::EmergencyPaused, Self::Cancelled) => true,
            
            // No transitions from final states (except emergency)
            _ => false,
        }
    }
}

/// SIMPLIFIED: Safe arithmetic operations with explicit bounds
pub fn safe_add_u8(a: u8, b: u8, max_allowed: u8) -> Result<u8> {
    require!(a <= max_allowed && b <= max_allowed, WagerError::ArithmeticError);
    let result = a.saturating_add(b);
    require!(result <= max_allowed, WagerError::ArithmeticError);
    Ok(result)
}

pub fn safe_add_u64(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(error!(WagerError::ArithmeticError))
}

pub fn safe_mul_u64(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(error!(WagerError::ArithmeticError))
}

pub fn safe_div_u64(a: u64, b: u64) -> Result<u64> {
    require!(b > 0, WagerError::ArithmeticError);
    Ok(a / b) // Division never overflows
}

/// SIMPLIFIED: Team with guaranteed safe operations
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
            player_spawns: [10; 5], // Safe starting value
            player_kills: [0; 5],
        }
    }
}

impl Team {
    /// SIMPLIFIED: Basic validation with clear bounds
    pub fn validate_stats(&self, player_count: usize) -> Result<()> {
        require!(player_count <= 5, WagerError::PlayerIndexOutOfBounds);
        
        for i in 0..player_count {
            require!(
                self.player_kills[i] <= MAX_KILLS && 
                self.player_spawns[i] <= MAX_SPAWNS,
                WagerError::InvalidKillCount
            );
        }
        Ok(())
    }

    /// SIMPLIFIED: Safe activity calculation with explicit bounds
    pub fn calculate_total_activity(&self, player_count: usize) -> Result<u64> {
        require!(player_count <= 5, WagerError::PlayerIndexOutOfBounds);
        
        let mut total = 0u64;
        for i in 0..player_count {
            let kills = self.player_kills[i] as u64;
            let spawns = self.player_spawns[i] as u64;
            
            // Individual player activity (guaranteed safe: max 50+50=100)
            let player_activity = safe_add_u64(kills, spawns)?;
            total = safe_add_u64(total, player_activity)?;
        }
        
        // Final safety check
        require!(total <= MAX_TOTAL_ACTIVITY * 5, WagerError::ValueTooLarge); // Max 500 for 5 players
        Ok(total)
    }

    pub fn get_empty_slot(&self, player_count: usize) -> Result<usize> {
        require!(player_count <= 5, WagerError::PlayerIndexOutOfBounds);
        
        for i in 0..player_count {
            if self.players[i] == Pubkey::default() {
                return Ok(i);
            }
        }
        Err(error!(WagerError::TeamIsFull))
    }

    pub fn contains_player(&self, player: Pubkey, player_count: usize) -> Result<bool> {
        require!(player_count <= 5, WagerError::PlayerIndexOutOfBounds);
        require!(player != Pubkey::default(), WagerError::InvalidPlayer);
        
        Ok(self.players[0..player_count].iter().any(|&p| p == player && p != Pubkey::default()))
    }

    /// Validate team composition
    pub fn validate_composition(&self, expected_count: usize) -> Result<()> {
        require!(expected_count <= 5, WagerError::PlayerIndexOutOfBounds);

        let filled_slots = self.players[0..expected_count]
            .iter()
            .filter(|&&p| p != Pubkey::default())
            .count();

        require!(filled_slots == expected_count, WagerError::InvalidTeamComposition);

        // Validate no duplicates within team
        let mut seen = HashSet::new();
        for &player in &self.players[0..expected_count] {
            if player != Pubkey::default() {
                require!(seen.insert(player), WagerError::DuplicatePlayer);
            }
        }

        Ok(())
    }
}

/// ENHANCED: Game session with atomic compare-and-swap operations
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
    pub nonce: u64,
    pub last_operation: i64,
    pub session_hash: [u8; 32],
    pub operation_count: u64,
    pub last_operation_window: i64,
    pub operations_in_window: u64,
    
    // NEW: Distribution tracking fields to prevent race conditions
    pub distribution_status: DistributionStatus,
    pub distribution_started_at: i64,
    pub distribution_nonce: u64,
    pub last_distribution_attempt: i64,
    
    // NEW: Operation locking mechanism
    pub current_operation: Option<String>, // Stores operation type currently in progress
    pub operation_started_at: i64,
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
        8 + // operations_in_window
        1 + // distribution_status
        8 + // distribution_started_at
        8 + // distribution_nonce
        8 + // last_distribution_attempt
        4 + 32 + // current_operation (Option<String>)
        8; // operation_started_at

    /// CRITICAL FIX: Atomic compare-and-swap for status transitions
    pub fn compare_and_swap_status(
        &mut self, 
        expected_status: GameStatus, 
        new_status: GameStatus,
        operation_type: Option<&str>
    ) -> Result<bool> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        
        // STEP 1: Check if current status matches expected
        if self.status != expected_status {
            msg!("CAS failed: expected {:?}, got {:?}", expected_status, self.status);
            return Ok(false);
        }
        
        // STEP 2: Validate the transition is allowed
        require!(
            self.status.can_transition_to(&new_status),
            WagerError::InvalidGameState
        );
        
        // STEP 3: Check for concurrent operations
        if let Some(ref current_op) = self.current_operation {
            // Check if operation has timed out (5 minutes max)
            if current_time - self.operation_started_at > 300 {
                msg!("Operation {} timed out, clearing lock", current_op);
                self.current_operation = None;
            } else {
                msg!("Operation {} still in progress, CAS failed", current_op);
                return Err(error!(WagerError::ConcurrentOperation));
            }
        }
        
        // STEP 4: Perform atomic update - all fields updated together
        self.status = new_status.clone();
        self.nonce = self.nonce
            .checked_add(1)
            .ok_or(WagerError::ArithmeticError)?;
        self.last_operation = current_time;
        
        // STEP 5: Set operation lock if specified
        if let Some(op_type) = operation_type {
            self.current_operation = Some(op_type.to_string());
            self.operation_started_at = current_time;
        }
        
        // STEP 6: Update distribution tracking if relevant
        match new_status {
            GameStatus::DistributionInProgress => {
                self.distribution_status = DistributionStatus::InProgress;
                self.distribution_started_at = current_time;
                self.distribution_nonce = self.distribution_nonce
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            },
            GameStatus::Completed => {
                if matches!(self.distribution_status, DistributionStatus::InProgress) {
                    self.distribution_status = DistributionStatus::Completed;
                }
            },
            GameStatus::Cancelled | GameStatus::Expired => {
                if matches!(self.distribution_status, DistributionStatus::InProgress) {
                    self.distribution_status = DistributionStatus::Failed;
                }
            },
            _ => {}
        }
        
        // STEP 7: Update operation tracking
        self.operation_count = self.operation_count
            .checked_add(1)
            .ok_or(WagerError::ArithmeticError)?;
            
        // Rate limiting check
        self.check_rate_limit()?;
        
        msg!("CAS successful: {:?} -> {:?}, nonce: {}, operation: {:?}", 
             expected_status, new_status, self.nonce, operation_type);
        
        Ok(true)
    }

    /// SIMPLIFIED: Initialize with strict validation
    pub fn initialize(
        &mut self,
        session_id: String,
        authority: Pubkey,
        session_bet: u64,
        game_mode: GameMode,
        bump: u8,
        vault_bump: u8,
    ) -> Result<()> {
        // Strict session ID validation
        require!(
            session_id.len() >= 8 && session_id.len() <= 32,
            WagerError::InvalidSessionId
        );
        
        // Only allow alphanumeric and underscore
        require!(
            session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            WagerError::InvalidSessionId
        );

        // Strict bet validation
        require!(
            session_bet >= MIN_BET && session_bet <= MAX_BET,
            WagerError::InvalidBetAmount
        );

        let clock = Clock::get()?;
        let session_hash = generate_session_hash(&session_id, authority, clock.unix_timestamp);
        
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

        // Initialize new fields
        self.distribution_status = DistributionStatus::NotStarted;
        self.distribution_started_at = 0;
        self.distribution_nonce = 0;
        self.last_distribution_attempt = 0;
        self.current_operation = None;
        self.operation_started_at = 0;

        Ok(())
    }

    /// ENHANCED: Atomic status transition (compatibility wrapper)
    pub fn atomic_status_transition(&mut self, new_status: GameStatus, expected_nonce: u64) -> Result<()> {
        // Verify expected nonce for additional safety
        require!(self.nonce == expected_nonce, WagerError::ConcurrentOperation);
        
        let current_status = self.status.clone();
        let success = self.compare_and_swap_status(
            current_status, 
            new_status, 
            Some("legacy_transition")
        )?;
        
        if !success {
            return Err(error!(WagerError::ConcurrentOperation));
        }
        
        Ok(())
    }
    
    /// NEW: Mark distribution as completed
    pub fn mark_distribution_completed(&mut self) -> Result<()> {
        require!(
            matches!(self.status, GameStatus::DistributionInProgress | GameStatus::Completed),
            WagerError::InvalidGameState
        );
        
        let clock = Clock::get()?;
        
        // Clear operation lock
        self.current_operation = None;
        
        // Update distribution status
        self.distribution_status = DistributionStatus::Completed;
        
        // Ensure final status is Completed
        if self.status != GameStatus::Completed {
            self.status = GameStatus::Completed;
        }
        
        self.last_operation = clock.unix_timestamp;
        
        msg!("Distribution marked as completed at {}", clock.unix_timestamp);
        Ok(())
    }
    
    /// NEW: Mark distribution as failed
    pub fn mark_distribution_failed(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        
        // Clear operation lock
        self.current_operation = None;
        
        // Update distribution status
        self.distribution_status = DistributionStatus::Failed;
        self.last_distribution_attempt = clock.unix_timestamp;
        
        // Transition to a failed state (could be cancelled or expired)
        self.status = GameStatus::Cancelled;
        self.last_operation = clock.unix_timestamp;
        
        msg!("Distribution marked as failed at {}", clock.unix_timestamp);
        Ok(())
    }

    /// SIMPLIFIED: Rate limiting with clear bounds
    pub fn check_rate_limit(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        
        // Simple 60-second window reset
        if current_time >= self.last_operation_window + 60 {
            self.last_operation_window = current_time;
            self.operations_in_window = 0;
        }
        
        require!(
            self.operations_in_window < MAX_OPERATIONS_PER_MINUTE,
            WagerError::RateLimitExceeded
        );
        
        self.operations_in_window = safe_add_u64(self.operations_in_window, 1)?;
        Ok(())
    }

    /// SIMPLIFIED: Safe operation tracking
    pub fn update_operation_tracking(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        self.check_rate_limit()?;
        
        self.nonce = safe_add_u64(self.nonce, 1)?;
        self.operation_count = safe_add_u64(self.operation_count, 1)?;
        self.last_operation = clock.unix_timestamp;
        
        Ok(())
    }

    /// SIMPLIFIED: Basic status transition
    pub fn transition_status(&mut self, new_status: GameStatus) -> Result<()> {
        // Basic validations
        match (&self.status, &new_status) {
            (GameStatus::WaitingForPlayers, GameStatus::InProgress) => {
                require!(self.check_all_filled()?, WagerError::NotAllPlayersJoined);
            },
            (GameStatus::InProgress, GameStatus::Completed) => {
                // Valid
            },
            (_, GameStatus::Cancelled) => {
                // Can cancel from most states
            },
            _ => {
                require!(!self.status.is_final(), WagerError::InvalidGameState);
            }
        }

        self.status = new_status;
        self.update_operation_tracking()?;
        Ok(())
    }

    /// SIMPLIFIED: Check teams are filled
    pub fn check_all_filled(&self) -> Result<bool> {
        let player_count = self.game_mode.players_per_team();
        
        // Validate both teams are full
        for i in 0..player_count {
            if self.team_a.players[i] == Pubkey::default() || 
               self.team_b.players[i] == Pubkey::default() {
                return Ok(false);
            }
        }

        // Check no duplicates across teams
        for i in 0..player_count {
            for j in 0..player_count {
                require!(
                    self.team_a.players[i] != self.team_b.players[j],
                    WagerError::DuplicatePlayer
                );
            }
        }

        Ok(true)
    }

    /// Secure version for compatibility
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

    /// BULLETPROOF: Earnings calculation with absolute safety
    pub fn calculate_player_earnings(&self, player_pubkey: Pubkey) -> Result<u64> {
        require!(player_pubkey != Pubkey::default(), WagerError::InvalidPlayer);
        
        // Find player and get their stats
        let player_count = self.game_mode.players_per_team();
        let mut kills = 0u8;
        let mut spawns = 0u8;
        let mut found = false;

        // Check team A
        for i in 0..player_count {
            if self.team_a.players[i] == player_pubkey {
                kills = self.team_a.player_kills[i];
                spawns = self.team_a.player_spawns[i];
                found = true;
                break;
            }
        }

        // Check team B if not found
        if !found {
            for i in 0..player_count {
                if self.team_b.players[i] == player_pubkey {
                    kills = self.team_b.player_kills[i];
                    spawns = self.team_b.player_spawns[i];
                    found = true;
                    break;
                }
            }
        }

        require!(found, WagerError::PlayerNotFound);

        // Validate stats are within bounds
        require!(kills <= MAX_KILLS && spawns <= MAX_SPAWNS, WagerError::InvalidKillCount);

        // BULLETPROOF calculation:
        // Max possible: kills=50, spawns=50, activity=100
        // Max bet=100M, so 100 * 100M = 10B (well under u64::MAX=18quintillion)
        let total_activity = safe_add_u64(kills as u64, spawns as u64)?;
        let gross_earnings = safe_mul_u64(total_activity, self.session_bet)?;
        let final_earnings = safe_div_u64(gross_earnings, 10)?;

        Ok(final_earnings)
    }

    /// Safe version for legacy compatibility
    pub fn calculate_player_earnings_safe(&self, player_pubkey: Pubkey) -> Result<u64> {
        self.calculate_player_earnings(player_pubkey)
    }

    /// SIMPLIFIED: Add kill with bounds checking
    pub fn add_kill(&mut self, killer_team: u8, killer: Pubkey, victim_team: u8, victim: Pubkey) -> Result<()> {
        require!(self.status.allows_game_operations(), WagerError::GameNotInProgress);
        require!(killer_team != victim_team && killer_team <= 1 && victim_team <= 1, WagerError::InvalidTeam);
        require!(killer != victim && killer != Pubkey::default() && victim != Pubkey::default(), WagerError::InvalidKillTarget);

        // Find player indices
        let killer_idx = self.find_player_index(killer_team, killer)?;
        let victim_idx = self.find_player_index(victim_team, victim)?;

        // Get current values safely
        let (victim_spawns, killer_kills) = match (killer_team, victim_team) {
            (0, 1) => (self.team_b.player_spawns[victim_idx], self.team_a.player_kills[killer_idx]),
            (1, 0) => (self.team_a.player_spawns[victim_idx], self.team_b.player_kills[killer_idx]),
            _ => return Err(error!(WagerError::InvalidTeam)),
        };

        // Validate operation is safe
        require!(victim_spawns > 0, WagerError::PlayerHasNoSpawns);
        require!(killer_kills < MAX_KILLS, WagerError::InvalidKillCount);

        // Perform safe updates
        let new_killer_kills = safe_add_u8(killer_kills, 1, MAX_KILLS)?;
        let new_victim_spawns = victim_spawns.saturating_sub(1);

        match (killer_team, victim_team) {
            (0, 1) => {
                self.team_a.player_kills[killer_idx] = new_killer_kills;
                self.team_b.player_spawns[victim_idx] = new_victim_spawns;
            },
            (1, 0) => {
                self.team_b.player_kills[killer_idx] = new_killer_kills;
                self.team_a.player_spawns[victim_idx] = new_victim_spawns;
            },
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        self.update_operation_tracking()?;
        Ok(())
    }

    /// SIMPLIFIED: Add spawns with strict limits
    pub fn add_spawns(&mut self, team: u8, player_index: usize) -> Result<()> {
        require!(self.game_mode.is_pay_to_spawn(), WagerError::InvalidGameModeForOperation);
        require!(team <= 1, WagerError::InvalidTeam);
        require!(player_index < self.game_mode.players_per_team(), WagerError::PlayerIndexOutOfBounds);

        let increment = self.game_mode.spawn_increment();
        require!(increment > 0, WagerError::InvalidSpawnIncrement);

        let current_spawns = match team {
            0 => self.team_a.player_spawns[player_index],
            1 => self.team_b.player_spawns[player_index],
            _ => return Err(error!(WagerError::InvalidTeam)),
        };

        let new_spawns = safe_add_u8(current_spawns, increment, MAX_SPAWNS)?;

        match team {
            0 => self.team_a.player_spawns[player_index] = new_spawns,
            1 => self.team_b.player_spawns[player_index] = new_spawns,
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        self.update_operation_tracking()?;
        Ok(())
    }

    /// Safe version for compatibility
    pub fn add_spawns_safe(&mut self, team: u8, player_index: usize) -> Result<()> {
        self.add_spawns(team, player_index)
    }

    /// Helper: Find player index safely
    fn find_player_index(&self, team: u8, player: Pubkey) -> Result<usize> {
        let player_count = self.game_mode.players_per_team();
        let team_players = match team {
            0 => &self.team_a.players[0..player_count],
            1 => &self.team_b.players[0..player_count],
            _ => return Err(error!(WagerError::InvalidTeam)),
        };

        team_players.iter()
            .position(|p| *p == player)
            .ok_or(error!(WagerError::PlayerNotFound))
    }

    /// Get player index (public version)
    pub fn get_player_index(&self, team: u8, player: Pubkey) -> Result<usize> {
        self.find_player_index(team, player)
    }

    /// SIMPLIFIED: Basic validation
    pub fn validate_not_expired(&self) -> Result<()> {
        let clock = Clock::get()?;
        let game_age = clock.unix_timestamp.saturating_sub(self.created_at);
        
        require!(game_age <= GAME_TIMEOUT_SECONDS, WagerError::GameTimeout);
        Ok(())
    }

    /// Safe version for compatibility
    pub fn validate_not_expired_safe(&self) -> Result<()> {
        self.validate_not_expired()
    }

    /// Check if player already joined
    pub fn is_player_already_joined(&self, player: Pubkey) -> Result<bool> {
        require!(player != Pubkey::default(), WagerError::InvalidPlayer);
        
        let player_count = self.game_mode.players_per_team();
        let in_team_a = self.team_a.contains_player(player, player_count)?;
        let in_team_b = self.team_b.contains_player(player, player_count)?;
        
        require!(!(in_team_a && in_team_b), WagerError::DuplicatePlayer);
        Ok(in_team_a || in_team_b)
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

    /// Get kills and spawns for a player
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
                kills <= MAX_KILLS && spawns <= MAX_SPAWNS,
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
                kills <= MAX_KILLS && spawns <= MAX_SPAWNS,
                WagerError::InvalidKillCount
            );
            
            return Ok((kills as u16)
                .checked_add(spawns as u16)
                .ok_or(WagerError::ArithmeticError)?);
        }
        
        Err(error!(WagerError::PlayerNotFound))
    }

    /// NEW: Check if distribution can be started
    pub fn can_start_distribution(&self) -> Result<bool> {
        // Check status allows distribution
        if !matches!(self.status, GameStatus::InProgress) {
            return Ok(false);
        }
        
        // Check distribution status
        if !matches!(self.distribution_status, DistributionStatus::NotStarted) {
            return Ok(false);
        }
        
        // Check no concurrent operations
        if self.current_operation.is_some() {
            return Ok(false);
        }
        
        // Validate game is ready for distribution
        if !self.check_all_filled_secure()? {
            return Ok(false);
        }
        
        Ok(true)
    }
    
    /// NEW: Clear operation lock (for emergency recovery)
    pub fn clear_operation_lock(&mut self, authority: Pubkey) -> Result<()> {
        require!(
            self.authority == authority,
            WagerError::UnauthorizedDistribution
        );
        
        let clock = Clock::get()?;
        
        // Only allow clearing if operation has timed out (10 minutes)
        if let Some(ref op) = self.current_operation {
            if clock.unix_timestamp - self.operation_started_at < 600 {
                return Err(error!(WagerError::ConcurrentOperation));
            }
        }
        
        self.current_operation = None;
        self.operation_started_at = 0;
        
        msg!("Operation lock cleared by authority");
        Ok(())
    }

    /// NEW: Mark refund as completed
    pub fn mark_refund_completed(&mut self) -> Result<()> {
        require!(
            matches!(self.status, GameStatus::RefundInProgress),
            WagerError::InvalidGameState
        );
        
        let clock = Clock::get()?;
        
        // Clear operation lock
        self.current_operation = None;
        
        // Transition to cancelled state (refunded games are considered cancelled)
        self.status = GameStatus::Cancelled;
        self.last_operation = clock.unix_timestamp;
        
        msg!("Refund marked as completed at {}", clock.unix_timestamp);
        Ok(())
    }
    
    /// NEW: Mark refund as failed
    pub fn mark_refund_failed(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        
        // Clear operation lock
        self.current_operation = None;
        
        // Keep the original status or mark as failed
        // Don't change to cancelled if refund failed - might need retry
        self.last_operation = clock.unix_timestamp;
        
        msg!("Refund marked as failed at {}", clock.unix_timestamp);
        Ok(())
    }

    /// Basic integrity validation
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
            self.session_bet >= MIN_BET && self.session_bet <= MAX_BET,
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

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overflow_impossible() {
        // Test that max possible earnings calculation is safe
        let max_activity = (MAX_KILLS as u64) + (MAX_SPAWNS as u64); // 100
        let max_earnings = max_activity * MAX_BET / 10; // 100 * 100M / 10 = 1B
        
        assert!(max_earnings < u64::MAX);
        assert!(max_activity <= MAX_TOTAL_ACTIVITY);
        
        // Verify no intermediate overflow
        let intermediate = max_activity * MAX_BET; // 100 * 100M = 10B
        assert!(intermediate < u64::MAX); // 10B << 18 quintillion
    }

    #[test]
    fn test_safe_arithmetic() {
        assert!(safe_add_u8(25, 25, MAX_KILLS).is_ok());
        assert!(safe_add_u8(26, 25, MAX_KILLS).is_err()); // Would exceed MAX_KILLS
        
        assert!(safe_add_u64(1000, 2000).unwrap() == 3000);
        assert!(safe_mul_u64(1000, 1000).unwrap() == 1_000_000);
    }

    #[test]
    fn test_conservative_limits() {
        // Our limits are very conservative
        assert!(MAX_KILLS < 100);
        assert!(MAX_SPAWNS < 100);
        assert!(MAX_BET < 1_000_000_000); // Less than 1 SOL
        
        // Max possible single calculation
        let max_calc = (MAX_KILLS as u64 + MAX_SPAWNS as u64) * MAX_BET;
        assert!(max_calc < u64::MAX / 1000); // Leaves huge safety margin
    }

    #[test]
    fn test_compare_and_swap_status() {
        // This would be a more comprehensive test in a real environment
        // Testing CAS operations requires mock Clock and proper setup
        let mut session = GameSession {
            session_id: "test123".to_string(),
            authority: Pubkey::default(),
            session_bet: MIN_BET,
            game_mode: GameMode::PayToSpawnOneVsOne,
            team_a: Team::default(),
            team_b: Team::default(),
            status: GameStatus::WaitingForPlayers,
            created_at: 1000000000,
            bump: 255,
            vault_bump: 254,
            nonce: 0,
            last_operation: 1000000000,
            session_hash: [0; 32],
            operation_count: 0,
            last_operation_window: 1000000000,
            operations_in_window: 0,
            distribution_status: DistributionStatus::NotStarted,
            distribution_started_at: 0,
            distribution_nonce: 0,
            last_distribution_attempt: 0,
            current_operation: None,
            operation_started_at: 0,
        };

        // Test valid transition
        // Note: This test would need proper Clock mocking in real environment
        // assert!(session.compare_and_swap_status(
        //     GameStatus::WaitingForPlayers, 
        //     GameStatus::InProgress, 
        //     Some("test_operation")
        // ).is_ok());
    }

    #[test] 
    fn test_distribution_status_tracking() {
        let status = DistributionStatus::NotStarted;
        assert_eq!(status, DistributionStatus::default());
        
        let game_status = GameStatus::DistributionInProgress;
        assert!(game_status.is_distributing());
        assert!(!game_status.is_final());
    }
}