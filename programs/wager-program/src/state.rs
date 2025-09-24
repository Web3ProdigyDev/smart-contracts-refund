use crate::errors::WagerError;
use crate::utils::{generate_secure_nonce, generate_session_hash};
use anchor_lang::prelude::*;
use std::collections::HashSet;

pub const MAX_KILLS: u8 = 50;
pub const MAX_SPAWNS: u8 = 50;
pub const MAX_BET: u64 = 100_000_000;
pub const MIN_BET: u64 = 1_000_000;
pub const GAME_TIMEOUT_SECONDS: i64 = 24 * 60 * 60;
pub const MAX_OPERATIONS_PER_MINUTE: u64 = 60;
pub const MAX_TOTAL_ACTIVITY: u64 = (MAX_KILLS as u64) + (MAX_SPAWNS as u64);
pub const SCALING_FACTOR: u64 = 1_000_000;

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
            Self::PayToSpawnOneVsOne => 3,
            Self::PayToSpawnThreeVsThree => 5,
            Self::PayToSpawnFiveVsFive => 7,
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

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq, Debug)]
pub enum GameStatus {
    WaitingForPlayers,
    InProgress,
    Completed,
    Cancelled,
    Expired,
    EmergencyPaused,
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
            (Self::WaitingForPlayers, Self::InProgress) => true,
            (Self::WaitingForPlayers, Self::Cancelled) => true,
            (Self::WaitingForPlayers, Self::Expired) => true,
            (Self::WaitingForPlayers, Self::RefundInProgress) => true,
            (Self::InProgress, Self::DistributionInProgress) => true,
            (Self::InProgress, Self::RefundInProgress) => true,
            (Self::InProgress, Self::Cancelled) => true,
            (Self::InProgress, Self::Expired) => true,
            (Self::DistributionInProgress, Self::Completed) => true,
            (Self::DistributionInProgress, Self::Cancelled) => true,
            (Self::RefundInProgress, Self::Cancelled) => true,
            (_, Self::EmergencyPaused) => true,
            (Self::EmergencyPaused, Self::InProgress) => true,
            (Self::EmergencyPaused, Self::Cancelled) => true,
            _ => false,
        }
    }
}

pub fn safe_add_u8(a: u8, b: u8, max_allowed: u8) -> Result<u8> {
    require!(
        a <= max_allowed && b <= max_allowed,
        WagerError::ArithmeticError
    );
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
    Ok(a / b)
}

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
            player_spawns: [10; 5],
            player_kills: [0; 5],
        }
    }
}

impl Team {
    pub fn validate_stats(&self, player_count: usize) -> Result<()> {
        require!(player_count <= 5, WagerError::PlayerIndexOutOfBounds);
        for i in 0..player_count {
            require!(
                self.player_kills[i] <= MAX_KILLS && self.player_spawns[i] <= MAX_SPAWNS,
                WagerError::InvalidKillCount
            );
        }
        Ok(())
    }

    pub fn calculate_total_activity(&self, player_count: usize) -> Result<u64> {
        require!(player_count <= 5, WagerError::PlayerIndexOutOfBounds);
        let mut total = 0u64;
        for i in 0..player_count {
            let kills = self.player_kills[i] as u64;
            let spawns = self.player_spawns[i] as u64;
            let player_activity = safe_add_u64(kills, spawns)?;
            total = safe_add_u64(total, player_activity)?;
        }
        require!(total <= MAX_TOTAL_ACTIVITY * 5, WagerError::ValueTooLarge);
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
        Ok(self.players[0..player_count]
            .iter()
            .any(|&p| p == player && p != Pubkey::default()))
    }

    pub fn validate_composition(&self, expected_count: usize) -> Result<()> {
        require!(expected_count <= 5, WagerError::PlayerIndexOutOfBounds);
        let filled_slots = self.players[0..expected_count]
            .iter()
            .filter(|&&p| p != Pubkey::default())
            .count();
        require!(
            filled_slots == expected_count,
            WagerError::InvalidTeamComposition
        );
        let mut seen = HashSet::new();
        for &player in &self.players[0..expected_count] {
            if player != Pubkey::default() {
                require!(seen.insert(player), WagerError::DuplicatePlayer);
            }
        }
        Ok(())
    }
}

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
    pub distribution_status: DistributionStatus,
    pub distribution_started_at: i64,
    pub distribution_nonce: u64,
    pub last_distribution_attempt: i64,
    pub current_operation: Option<String>,
    pub operation_started_at: i64,
    pub locked: bool,
}

impl GameSession {
    pub const MAX_SIZE: usize = 8
        + 4
        + 32
        + 32
        + 8
        + 1
        + (32 * 5 + 8 + 5 + 5)
        + (32 * 5 + 8 + 5 + 5)
        + 1
        + 8
        + 1
        + 1
        + 8
        + 8
        + 32
        + 8
        + 8
        + 8
        + 1
        + 8
        + 8
        + 8
        + 4
        + 32
        + 8
        + 1;

    pub fn compare_and_swap_status(
        &mut self,
        expected_status: GameStatus,
        expected_nonce: u64,
        new_status: GameStatus,
        operation_type: Option<&str>,
    ) -> Result<bool> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;

        // Check if account is already locked
        if self.locked {
            msg!(
                "CAS failed: account is locked, operation={:?}",
                self.current_operation
            );
            return Err(error!(WagerError::ConcurrentOperation));
        }

        // Lock the account FIRST
        self.locked = true;
        self.current_operation = operation_type.map(|s| s.to_string());
        self.operation_started_at = current_time;

        // Check status and nonce after locking
        if self.status != expected_status || self.nonce != expected_nonce {
            msg!(
                "CAS failed: expected status={:?}, nonce={}, got status={:?}, nonce={}",
                expected_status,
                expected_nonce,
                self.status,
                self.nonce
            );
            self.locked = false;
            self.current_operation = None;
            self.operation_started_at = 0;
            return Ok(false);
        }

        require!(
            self.status.can_transition_to(&new_status),
            WagerError::InvalidGameState
        );

        // Perform atomic update
        self.status = new_status.clone();
        self.nonce = self
            .nonce
            .checked_add(1)
            .ok_or(WagerError::ArithmeticError)?;
        self.last_operation = current_time;

        // Update distribution tracking if relevant
        match new_status {
            GameStatus::DistributionInProgress => {
                self.distribution_status = DistributionStatus::InProgress;
                self.distribution_started_at = current_time;
                self.distribution_nonce = self
                    .distribution_nonce
                    .checked_add(1)
                    .ok_or(WagerError::ArithmeticError)?;
            }
            GameStatus::Completed => {
                if matches!(self.distribution_status, DistributionStatus::InProgress) {
                    self.distribution_status = DistributionStatus::Completed;
                    self.locked = false;
                    self.current_operation = None;
                    self.operation_started_at = 0;
                }
            }
            GameStatus::Cancelled | GameStatus::Expired => {
                if matches!(self.distribution_status, DistributionStatus::InProgress) {
                    self.distribution_status = DistributionStatus::Failed;
                    self.locked = false;
                    self.current_operation = None;
                    self.operation_started_at = 0;
                }
            }
            _ => {}
        }

        self.operation_count = self
            .operation_count
            .checked_add(1)
            .ok_or(WagerError::ArithmeticError)?;
        self.check_rate_limit()?;

        // Unlock after successful update unless it's a long-running operation
        if !matches!(
            new_status,
            GameStatus::DistributionInProgress | GameStatus::RefundInProgress
        ) {
            self.locked = false;
            self.current_operation = None;
            self.operation_started_at = 0;
        }

        msg!(
            "CAS successful: session_id={}, authority={}, status: {:?} -> {:?}, nonce: {}, operation: {:?}",
            self.session_id,
            self.authority,
            expected_status,
            new_status,
            self.nonce,
            operation_type
        );
        Ok(true)
    }

    pub fn initialize(
        &mut self,
        session_id: String,
        authority: Pubkey,
        session_bet: u64,
        game_mode: GameMode,
        bump: u8,
        vault_bump: u8,
    ) -> Result<()> {
        require!(
            session_id.len() >= 8 && session_id.len() <= 32,
            WagerError::InvalidSessionId
        );
        require!(
            session_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
            WagerError::InvalidSessionId
        );
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
        self.distribution_status = DistributionStatus::NotStarted;
        self.distribution_started_at = 0;
        self.distribution_nonce = 0;
        self.last_distribution_attempt = 0;
        self.current_operation = None;
        self.operation_started_at = 0;
        self.locked = false;
        Ok(())
    }

    pub fn atomic_status_transition(
        &mut self,
        new_status: GameStatus,
        expected_nonce: u64,
    ) -> Result<()> {
        let current_status = self.status.clone();
        let success = self.compare_and_swap_status(
            current_status,
            expected_nonce,
            new_status,
            Some("legacy_transition"),
        )?;
        if !success {
            return Err(error!(WagerError::ConcurrentOperation));
        }
        Ok(())
    }

    pub fn mark_distribution_completed(&mut self) -> Result<()> {
        require!(
            matches!(
                self.status,
                GameStatus::DistributionInProgress | GameStatus::Completed
            ),
            WagerError::InvalidGameState
        );
        let clock = Clock::get()?;
        self.current_operation = None;
        self.locked = false;
        self.distribution_status = DistributionStatus::Completed;
        if self.status != GameStatus::Completed {
            self.status = GameStatus::Completed;
        }
        self.last_operation = clock.unix_timestamp;
        msg!(
            "Distribution marked as completed at {} for session_id={}",
            clock.unix_timestamp,
            self.session_id
        );
        Ok(())
    }

    pub fn mark_distribution_failed(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        self.current_operation = None;
        self.locked = false;
        self.distribution_status = DistributionStatus::Failed;
        self.last_distribution_attempt = clock.unix_timestamp;
        self.status = GameStatus::Cancelled;
        self.last_operation = clock.unix_timestamp;
        msg!(
            "Distribution marked as failed at {} for session_id={}",
            clock.unix_timestamp,
            self.session_id
        );
        Ok(())
    }

    pub fn check_rate_limit(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;
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

    pub fn update_operation_tracking(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        self.check_rate_limit()?;
        self.nonce = safe_add_u64(self.nonce, 1)?;
        self.operation_count = safe_add_u64(self.operation_count, 1)?;
        self.last_operation = clock.unix_timestamp;
        Ok(())
    }

    pub fn transition_status(&mut self, new_status: GameStatus) -> Result<()> {
        match (&self.status, &new_status) {
            (GameStatus::WaitingForPlayers, GameStatus::InProgress) => {
                require!(self.check_all_filled()?, WagerError::NotAllPlayersJoined);
            }
            (GameStatus::InProgress, GameStatus::Completed) => {}
            (_, GameStatus::Cancelled) => {}
            _ => {
                require!(!self.status.is_final(), WagerError::InvalidGameState);
            }
        }
        self.status = new_status;
        self.update_operation_tracking()?;
        Ok(())
    }

    pub fn check_all_filled(&self) -> Result<bool> {
        let player_count = self.game_mode.players_per_team();
        for i in 0..player_count {
            if self.team_a.players[i] == Pubkey::default()
                || self.team_b.players[i] == Pubkey::default()
            {
                return Ok(false);
            }
        }
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

    pub fn check_all_filled_secure(&self) -> Result<bool> {
        let player_count = self.game_mode.players_per_team();
        self.team_a.validate_composition(player_count)?;
        self.team_b.validate_composition(player_count)?;
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

    pub fn calculate_player_earnings(&self, player_pubkey: Pubkey) -> Result<u64> {
        require!(
            player_pubkey != Pubkey::default(),
            WagerError::InvalidPlayer
        );
        let player_count = self.game_mode.players_per_team();
        let mut kills = 0u8;
        let mut spawns = 0u8;
        let mut found = false;
        for i in 0..player_count {
            if self.team_a.players[i] == player_pubkey {
                kills = self.team_a.player_kills[i];
                spawns = self.team_a.player_spawns[i];
                found = true;
                break;
            }
        }
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
        require!(
            kills <= MAX_KILLS && spawns <= MAX_SPAWNS,
            WagerError::InvalidKillCount
        );
        let total_activity = safe_add_u64(kills as u64, spawns as u64)?;
        require!(
            total_activity <= MAX_TOTAL_ACTIVITY,
            WagerError::ValueTooLarge
        );
        let scaled_activity = safe_mul_u64(total_activity, SCALING_FACTOR)?;
        let gross_earnings = safe_mul_u64(scaled_activity, self.session_bet)?;
        let final_earnings = safe_div_u64(gross_earnings, SCALING_FACTOR * 10)?;
        let max_earnings = safe_mul_u64(MAX_TOTAL_ACTIVITY, MAX_BET)?;
        require!(final_earnings <= max_earnings, WagerError::ValueTooLarge);
        Ok(final_earnings)
    }

    pub fn calculate_player_earnings_safe(&self, player_pubkey: Pubkey) -> Result<u64> {
        self.calculate_player_earnings(player_pubkey)
    }

    pub fn add_kill(
        &mut self,
        killer_team: u8,
        killer: Pubkey,
        victim_team: u8,
        victim: Pubkey,
    ) -> Result<()> {
        require!(
            self.status.allows_game_operations(),
            WagerError::GameNotInProgress
        );
        require!(
            killer_team != victim_team && killer_team <= 1 && victim_team <= 1,
            WagerError::InvalidTeam
        );
        require!(
            killer != victim && killer != Pubkey::default() && victim != Pubkey::default(),
            WagerError::InvalidKillTarget
        );
        let killer_idx = self.find_player_index(killer_team, killer)?;
        let victim_idx = self.find_player_index(victim_team, victim)?;
        let (victim_spawns, killer_kills) = match (killer_team, victim_team) {
            (0, 1) => (
                self.team_b.player_spawns[victim_idx],
                self.team_a.player_kills[killer_idx],
            ),
            (1, 0) => (
                self.team_a.player_spawns[victim_idx],
                self.team_b.player_kills[killer_idx],
            ),
            _ => return Err(error!(WagerError::InvalidTeam)),
        };
        require!(victim_spawns > 0, WagerError::PlayerHasNoSpawns);
        require!(killer_kills < MAX_KILLS, WagerError::InvalidKillCount);
        let new_killer_kills = safe_add_u8(killer_kills, 1, MAX_KILLS)?;
        let new_victim_spawns = victim_spawns.saturating_sub(1);
        match (killer_team, victim_team) {
            (0, 1) => {
                self.team_a.player_kills[killer_idx] = new_killer_kills;
                self.team_b.player_spawns[victim_idx] = new_victim_spawns;
            }
            (1, 0) => {
                self.team_b.player_kills[killer_idx] = new_killer_kills;
                self.team_a.player_spawns[victim_idx] = new_victim_spawns;
            }
            _ => return Err(error!(WagerError::InvalidTeam)),
        }
        self.update_operation_tracking()?;
        Ok(())
    }

    pub fn add_spawns(&mut self, team: u8, player_index: usize) -> Result<()> {
        require!(
            self.game_mode.is_pay_to_spawn(),
            WagerError::InvalidGameModeForOperation
        );
        require!(team <= 1, WagerError::InvalidTeam);
        require!(
            player_index < self.game_mode.players_per_team(),
            WagerError::PlayerIndexOutOfBounds
        );
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

    pub fn add_spawns_safe(&mut self, team: u8, player_index: usize) -> Result<()> {
        self.add_spawns(team, player_index)
    }

    fn find_player_index(&self, team: u8, player: Pubkey) -> Result<usize> {
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

    pub fn get_player_index(&self, team: u8, player: Pubkey) -> Result<usize> {
        self.find_player_index(team, player)
    }

    pub fn validate_not_expired(&self) -> Result<()> {
        let clock = Clock::get()?;
        let game_age = clock.unix_timestamp.saturating_sub(self.created_at);
        require!(game_age <= GAME_TIMEOUT_SECONDS, WagerError::GameTimeout);
        Ok(())
    }

    pub fn validate_not_expired_safe(&self) -> Result<()> {
        self.validate_not_expired()
    }

    pub fn is_player_already_joined(&self, player: Pubkey) -> Result<bool> {
        require!(player != Pubkey::default(), WagerError::InvalidPlayer);
        let player_count = self.game_mode.players_per_team();
        let in_team_a = self.team_a.contains_player(player, player_count)?;
        let in_team_b = self.team_b.contains_player(player, player_count)?;
        require!(!(in_team_a && in_team_b), WagerError::DuplicatePlayer);
        Ok(in_team_a || in_team_b)
    }

    pub fn get_player_empty_slot(&self, team: u8) -> Result<usize> {
        let player_count = self.game_mode.players_per_team();
        match team {
            0 => self.team_a.get_empty_slot(player_count),
            1 => self.team_b.get_empty_slot(player_count),
            _ => Err(error!(WagerError::InvalidTeam)),
        }
    }

    pub fn is_pay_to_spawn(&self) -> bool {
        self.game_mode.is_pay_to_spawn()
    }

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

    pub fn get_kills_and_spawns(&self, player_pubkey: Pubkey) -> Result<u16> {
        require!(
            player_pubkey != Pubkey::default(),
            WagerError::InvalidPlayer
        );
        let player_count = self.game_mode.players_per_team();
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

    pub fn can_start_distribution(&self) -> Result<bool> {
        if !matches!(self.status, GameStatus::InProgress) {
            return Ok(false);
        }
        if !matches!(self.distribution_status, DistributionStatus::NotStarted) {
            return Ok(false);
        }
        if self.current_operation.is_some() || self.locked {
            return Ok(false);
        }
        if !self.check_all_filled_secure()? {
            return Ok(false);
        }
        Ok(true)
    }

    pub fn clear_operation_lock(&mut self, authority: Pubkey) -> Result<()> {
        require!(
            self.authority == authority,
            WagerError::UnauthorizedDistribution
        );
        let clock = Clock::get()?;
        let current_time = clock.unix_timestamp;

        // Define different timeouts for different operations
        let timeout_duration = match self.current_operation.as_deref() {
            Some(op) if op.contains("distribution") || op.contains("refund") => 300, // 5 minutes for critical operations
            _ => 120, // 2 minutes for others
        };

        // Check if lock is active and not timed out
        if self.locked && current_time - self.operation_started_at < timeout_duration {
            msg!(
                "Cannot clear lock: operation={:?} in progress, time remaining={}s",
                self.current_operation,
                timeout_duration - (current_time - self.operation_started_at)
            );
            return Err(error!(WagerError::ConcurrentOperation));
        }

        // Clear the lock
        self.locked = false;
        self.current_operation = None;
        self.operation_started_at = 0;
        msg!(
            "Operation lock cleared by authority at {} for session_id={}",
            current_time,
            self.session_id
        );
        Ok(())
    }

    pub fn mark_refund_completed(&mut self) -> Result<()> {
        require!(
            matches!(self.status, GameStatus::RefundInProgress),
            WagerError::InvalidGameState
        );
        let clock = Clock::get()?;
        self.current_operation = None;
        self.locked = false;
        self.status = GameStatus::Cancelled;
        self.last_operation = clock.unix_timestamp;
        msg!(
            "Refund marked as completed at {} for session_id={}",
            clock.unix_timestamp,
            self.session_id
        );
        Ok(())
    }

    pub fn mark_refund_failed(&mut self) -> Result<()> {
        let clock = Clock::get()?;
        self.current_operation = None;
        self.locked = false;
        self.last_operation = clock.unix_timestamp;
        msg!(
            "Refund marked as failed at {} for session_id={}",
            clock.unix_timestamp,
            self.session_id
        );
        Ok(())
    }

    pub fn validate_integrity(&self) -> Result<()> {
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
        require!(
            self.created_at > 0 && self.last_operation >= self.created_at,
            WagerError::InvalidTimestamp
        );
        let expected_hash =
            generate_session_hash(&self.session_id, self.authority, self.created_at);
        require!(
            self.session_hash == expected_hash,
            WagerError::SessionIdCollision
        );
        require!(
            !self.locked || self.current_operation.is_some(),
            WagerError::GameDataCorruption
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_overflow_impossible() {
        let max_activity = (MAX_KILLS as u64) + (MAX_SPAWNS as u64);
        let max_earnings = max_activity * MAX_BET / 10;
        assert!(max_earnings < u64::MAX);
        assert!(max_activity <= MAX_TOTAL_ACTIVITY);
        let intermediate = max_activity * MAX_BET;
        assert!(intermediate < u64::MAX);
    }

    #[test]
    fn test_safe_arithmetic() {
        assert!(safe_add_u8(25, 25, MAX_KILLS).is_ok());
        assert!(safe_add_u8(26, 25, MAX_KILLS).is_err());
        assert!(safe_add_u64(1000, 2000).unwrap() == 3000);
        assert!(safe_mul_u64(1000, 1000).unwrap() == 1_000_000);
    }

    #[test]
    fn test_conservative_limits() {
        assert!(MAX_KILLS < 100);
        assert!(MAX_SPAWNS < 100);
        assert!(MAX_BET < 1_000_000_000);
        let max_calc = (MAX_KILLS as u64 + MAX_SPAWNS as u64) * MAX_BET;
        assert!(max_calc < u64::MAX / 1000);
    }

    #[test]
    fn test_compare_and_swap_status_concurrent_lock() {
        let mut game_session = GameSession {
            session_id: "test_session".to_string(),
            authority: Pubkey::new_unique(),
            session_bet: MIN_BET,
            game_mode: GameMode::WinnerTakesAllOneVsOne,
            team_a: Team::default(),
            team_b: Team::default(),
            status: GameStatus::WaitingForPlayers,
            created_at: 1234567890,
            bump: 255,
            vault_bump: 254,
            nonce: 100,
            last_operation: 1234567890,
            session_hash: [0u8; 32],
            operation_count: 0,
            last_operation_window: 1234567890,
            operations_in_window: 0,
            distribution_status: DistributionStatus::NotStarted,
            distribution_started_at: 0,
            distribution_nonce: 0,
            last_distribution_attempt: 0,
            current_operation: None,
            operation_started_at: 0,
            locked: false,
        };

        // Test successful CAS
        let result = game_session.compare_and_swap_status(
            GameStatus::WaitingForPlayers,
            100,
            GameStatus::InProgress,
            Some("test_transition"),
        );
        assert!(result.is_ok());
        assert!(result.unwrap());
        assert_eq!(game_session.status, GameStatus::InProgress);
        assert_eq!(game_session.nonce, 101);
        assert!(game_session.locked);
        assert_eq!(
            game_session.current_operation,
            Some("test_transition".to_string())
        );

        // Test concurrent CAS attempt (should fail due to lock)
        let result = game_session.compare_and_swap_status(
            GameStatus::InProgress,
            101,
            GameStatus::Completed,
            Some("test_concurrent"),
        );
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            WagerError::ConcurrentOperation.to_string()
        );
        assert_eq!(game_session.status, GameStatus::InProgress);
        assert_eq!(game_session.nonce, 101);

        // Test failed CAS due to wrong status
        game_session.locked = false;
        game_session.current_operation = None;
        game_session.operation_started_at = 0;
        let result = game_session.compare_and_swap_status(
            GameStatus::WaitingForPlayers,
            101,
            GameStatus::Completed,
            Some("test_fail"),
        );
        assert!(result.is_ok());
        assert!(!result.unwrap());
        assert_eq!(game_session.status, GameStatus::InProgress);
        assert_eq!(game_session.nonce, 101);
        assert!(!game_session.locked);
    }

    #[test]
    fn test_clear_operation_lock_timeout() {
        let mut game_session = GameSession {
            session_id: "test_session".to_string(),
            authority: Pubkey::new_unique(),
            session_bet: MIN_BET,
            game_mode: GameMode::WinnerTakesAllOneVsOne,
            team_a: Team::default(),
            team_b: Team::default(),
            status: GameStatus::DistributionInProgress,
            created_at: 1234567890,
            bump: 255,
            vault_bump: 254,
            nonce: 100,
            last_operation: 1234567890,
            session_hash: [0u8; 32],
            operation_count: 0,
            last_operation_window: 1234567890,
            operations_in_window: 0,
            distribution_status: DistributionStatus::InProgress,
            distribution_started_at: 1234567890,
            distribution_nonce: 0,
            last_distribution_attempt: 0,
            current_operation: Some("distribution".to_string()),
            operation_started_at: 1234567890,
            locked: true,
        };

        // Test clearing lock before timeout (should fail)
        let result = game_session.clear_operation_lock(game_session.authority);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            WagerError::ConcurrentOperation.to_string()
        );

        // Simulate timeout (300 seconds for distribution)
        game_session.operation_started_at = 1234567890 - 300;
        let result = game_session.clear_operation_lock(game_session.authority);
        assert!(result.is_ok());
        assert!(!game_session.locked);
        assert_eq!(game_session.current_operation, None);
        assert_eq!(game_session.operation_started_at, 0);

        // Test unauthorized clear attempt
        let result = game_session.clear_operation_lock(Pubkey::new_unique());
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            WagerError::UnauthorizedDistribution.to_string()
        );
    }
}
