//! State accounts for the betting program - SECURITY HARDENED VERSION WITH FIXES
use crate::errors::WagerError;
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
}

/// Status of a game session
#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq)]
pub enum GameStatus {
    WaitingForPlayers, // Waiting for players to join
    InProgress,        // Game is active with all players joined
    Completed,         // Game has finished and rewards distributed
}

impl Default for GameStatus {
    fn default() -> Self {
        Self::WaitingForPlayers
    }
}

/// Represents a team in the game - OPTIMIZED FOR SMALLER STACK USAGE
#[derive(AnchorSerialize, AnchorDeserialize, Clone)]
pub struct Team {
    pub players: [Pubkey; 5],    // Array of player public keys
    pub total_bet: u64,          // Total amount bet by team (in lamports)  
    pub player_spawns: [u8; 5],  // CHANGED: u8 instead of u16 (255 max spawns)
    pub player_kills: [u8; 5],   // CHANGED: u8 instead of u16 (255 max kills)
}

impl Default for Team {
    fn default() -> Self {
        Self {
            players: [Pubkey::default(); 5],
            total_bet: 0,
            player_spawns: [0; 5],
            player_kills: [0; 5],
        }
    }
}

impl Team {
    /// Finds the first empty slot in the team, if available
    pub fn get_empty_slot(&self, player_count: usize) -> Result<usize> {
        self.players
            .iter()
            .enumerate()
            .find(|(i, player)| **player == Pubkey::default() && *i < player_count)
            .map(|(i, _)| i)
            .ok_or_else(|| error!(WagerError::TeamIsFull))
    }
    
    /// CRITICAL FIX: Check if a player is already in this team
    pub fn contains_player(&self, player: Pubkey, player_count: usize) -> bool {
        self.players[0..player_count].iter().any(|&p| p == player && p != Pubkey::default())
    }
}

/// Represents a game session - OPTIMIZED SPACE CALCULATION
#[account]
pub struct GameSession {
    pub session_id: String,  // 4 + variable (max 32 bytes reasonable)
    pub authority: Pubkey,   // 32 bytes
    pub session_bet: u64,    // 8 bytes
    pub game_mode: GameMode, // 1 byte (enum)
    pub team_a: Team,        // 32*5 + 8 + 5 + 5 = 178 bytes
    pub team_b: Team,        // 32*5 + 8 + 5 + 5 = 178 bytes  
    pub status: GameStatus,  // 1 byte (enum)
    pub created_at: i64,     // 8 bytes
    pub bump: u8,            // 1 byte
    pub vault_bump: u8,      // 1 byte
}

// Total: ~450 bytes instead of previous larger size

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
        1; // vault_bump
        // Total: ~450 bytes

    /// Gets an empty slot for a player in the specified team
    pub fn get_player_empty_slot(&self, team: u8) -> Result<usize> {
        let player_count = self.game_mode.players_per_team();
        match team {
            0 => self.team_a.get_empty_slot(player_count),
            1 => self.team_b.get_empty_slot(player_count),
            _ => Err(error!(WagerError::InvalidTeam)),
        }
    }

    /// CRITICAL FIX: Replace error-matching logic with direct validation
    pub fn check_all_filled_secure(&self) -> Result<bool> {
        let player_count = self.game_mode.players_per_team();
        
        let team_a_full = self.team_a.players[0..player_count]
            .iter()
            .all(|p| *p != Pubkey::default());
            
        let team_b_full = self.team_b.players[0..player_count]
            .iter()
            .all(|p| *p != Pubkey::default());
        
        Ok(team_a_full && team_b_full)
    }

    /// Legacy method for backward compatibility - now uses secure version
    pub fn check_all_filled(&self) -> Result<bool> {
        self.check_all_filled_secure()
    }

    pub fn is_pay_to_spawn(&self) -> bool {
        matches!(
            self.game_mode,
            GameMode::PayToSpawnOneVsOne
                | GameMode::PayToSpawnThreeVsThree
                | GameMode::PayToSpawnFiveVsFive
        )
    }

    pub fn get_all_players(&self) -> Vec<Pubkey> {
        let mut players = Vec::with_capacity(10);
        players.extend_from_slice(&self.team_a.players);
        players.extend_from_slice(&self.team_b.players);
        players
    }

    pub fn get_player_index(&self, team: u8, player: Pubkey) -> Result<usize> {
        let player_count = self.game_mode.players_per_team();
        match team {
            0 => self.team_a.players[0..player_count]
                .iter()
                .position(|p| *p == player)
                .ok_or(error!(WagerError::PlayerNotFound)),
            1 => self.team_b.players[0..player_count]
                .iter()
                .position(|p| *p == player)
                .ok_or(error!(WagerError::PlayerNotFound)),
            _ => Err(error!(WagerError::InvalidTeam)),
        }
    }

    /// Gets the kill and spawn sum for a player
    pub fn get_kills_and_spawns(&self, player_pubkey: Pubkey) -> Result<u16> {
        let player_count = self.game_mode.players_per_team();
        
        // Check team A
        if let Some(idx) = self.team_a.players[0..player_count].iter().position(|p| *p == player_pubkey) {
            return Ok(self.team_a.player_kills[idx] as u16 + self.team_a.player_spawns[idx] as u16);
        }
        
        // Check team B
        if let Some(idx) = self.team_b.players[0..player_count].iter().position(|p| *p == player_pubkey) {
            return Ok(self.team_b.player_kills[idx] as u16 + self.team_b.player_spawns[idx] as u16);
        }
        
        Err(error!(WagerError::PlayerNotFound))
    }

    pub fn add_kill(
        &mut self,
        killer_team: u8,
        killer: Pubkey,
        victim_team: u8,
        victim: Pubkey,
    ) -> Result<()> {
        require!(
            self.status == GameStatus::InProgress,
            WagerError::GameNotInProgress
        );

        // CRITICAL FIX: Prevent same-team kills
        require!(
            killer_team != victim_team,
            WagerError::InvalidKillTarget
        );

        let killer_idx = self.get_player_index(killer_team, killer)?;
        let victim_idx = self.get_player_index(victim_team, victim)?;

        // CRITICAL FIX: Check victim has spawns before kill
        match victim_team {
            0 => {
                require!(
                    self.team_a.player_spawns[victim_idx] > 0,
                    WagerError::PlayerHasNoSpawns
                );
                self.team_a.player_spawns[victim_idx] -= 1;
            }
            1 => {
                require!(
                    self.team_b.player_spawns[victim_idx] > 0,
                    WagerError::PlayerHasNoSpawns
                );
                self.team_b.player_spawns[victim_idx] -= 1;
            }
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        // Add kill to killer (with overflow protection)
        match killer_team {
            0 => {
                self.team_a.player_kills[killer_idx] = self.team_a.player_kills[killer_idx]
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            1 => {
                self.team_b.player_kills[killer_idx] = self.team_b.player_kills[killer_idx]
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            _ => return Err(error!(WagerError::InvalidTeam)),
        }

        Ok(())
    }

    /// CRITICAL FIX: Standardized safe spawn addition with consistent validation
    pub fn add_spawns_safe(&mut self, team: u8, player_index: usize) -> Result<()> {
        const MAX_SPAWNS: u8 = 100;
        const SPAWN_INCREMENT: u8 = 10;
        
        match team {
            0 => {
                let current = self.team_a.player_spawns[player_index];
                // Check if adding spawn increment would exceed limit
                require!(
                    current <= MAX_SPAWNS.saturating_sub(SPAWN_INCREMENT),
                    WagerError::SpawnLimitExceeded
                );
                // Use checked_add for additional safety
                self.team_a.player_spawns[player_index] = current
                    .checked_add(SPAWN_INCREMENT)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            1 => {
                let current = self.team_b.player_spawns[player_index];
                // Check if adding spawn increment would exceed limit
                require!(
                    current <= MAX_SPAWNS.saturating_sub(SPAWN_INCREMENT),
                    WagerError::SpawnLimitExceeded
                );
                // Use checked_add for additional safety
                self.team_b.player_spawns[player_index] = current
                    .checked_add(SPAWN_INCREMENT)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            _ => return Err(error!(WagerError::InvalidTeam)),
        }
        Ok(())
    }

    /// Legacy method - redirects to safe version for backward compatibility
    pub fn add_spawns(&mut self, team: u8, player_index: usize) -> Result<()> {
        self.add_spawns_safe(team, player_index)
    }
    
    /// CRITICAL FIX: Check if player is already joined to prevent duplicates
    pub fn is_player_already_joined(&self, player: Pubkey) -> Result<bool> {
        let player_count = self.game_mode.players_per_team();
        
        Ok(self.team_a.contains_player(player, player_count) || 
           self.team_b.contains_player(player, player_count))
    }
    
    /// CRITICAL FIX: Safe timestamp arithmetic with negative check prevention
    pub fn validate_not_expired_safe(&self) -> Result<()> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
        
        // Prevent negative timestamps and ensure created_at is valid
        require!(
            current_time >= self.created_at && self.created_at > 0,
            WagerError::InvalidTimestamp
        );
        
        // Use saturating_sub to prevent underflow
        let game_age = current_time.saturating_sub(self.created_at);
        const GAME_TIMEOUT_SECONDS: i64 = 24 * 60 * 60; // 24 hours
        
        require!(
            game_age <= GAME_TIMEOUT_SECONDS,
            WagerError::GameTimeout
        );
        
        Ok(())
    }
    
    /// Legacy method for backward compatibility - now uses safe version
    pub fn validate_not_expired(&self) -> Result<()> {
        self.validate_not_expired_safe()
    }
    
    /// CRITICAL FIX: Validate session can transition to new status
    pub fn validate_status_transition(&self, new_status: GameStatus) -> Result<()> {
        match (&self.status, &new_status) {
            (GameStatus::WaitingForPlayers, GameStatus::InProgress) => {
                require!(self.check_all_filled_secure()?, WagerError::NotAllPlayersJoined);
            },
            (GameStatus::InProgress, GameStatus::Completed) => {
                // Valid transition
            },
            (GameStatus::Completed, _) => {
                return Err(error!(WagerError::AlreadyCompleted));
            },
            _ => return Err(error!(WagerError::InvalidGameState)),
        }
        Ok(())
    }
}

