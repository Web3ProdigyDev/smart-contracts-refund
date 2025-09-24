use anchor_lang::prelude::*;

#[error_code]
pub enum WagerError {
    #[msg("Game session is not in the correct state")]
    InvalidGameState,

    #[msg("Invalid team selection. Team must be 0 or 1")]
    InvalidTeamSelection,

    #[msg("Team is already full")]
    TeamIsFull,

    #[msg("Insufficient funds to join the game")]
    InsufficientFunds,

    #[msg("Invalid number of players for this game mode")]
    InvalidPlayerCount,

    #[msg("All players not joined")]
    NotAllPlayersJoined,

    #[msg("Game is not in completed state")]
    GameNotCompleted,

    #[msg("Only the game authority can distribute winnings")]
    UnauthorizedDistribution,

    #[msg("Only the game authority can pause the game")]
    UnauthorizedPause,

    #[msg("Invalid winning team selection")]
    InvalidWinningTeam,

    #[msg("Failed to calculate total pot due to arithmetic overflow")]
    TotalPotCalculationError,

    #[msg("No winners found in the winning team")]
    NoWinnersFound,

    #[msg("Failed to calculate per-player winnings")]
    WinningsCalculationError,

    #[msg("Failed to distribute all funds from game session")]
    IncompleteDistribution,

    #[msg("Invalid team")]
    InvalidTeam,

    #[msg("Player account not found in winners")]
    PlayerAccountNotFound,

    #[msg("Invalid winning team selection")]
    InvalidWinner,

    #[msg("Arithmetic error")]
    ArithmeticError,

    #[msg("Invalid mint address provided")]
    InvalidMint,

    #[msg("Invalid remaining accounts provided")]
    InvalidRemainingAccounts,

    #[msg("Invalid winner token account owner")]
    InvalidWinnerTokenAccount,

    #[msg("Invalid token mint")]
    InvalidTokenMint,

    #[msg("Invalid spawns")]
    InvalidSpawns,

    #[msg("Unauthorized kill")]
    UnauthorizedKill,

    #[msg("Unauthorized pay to spawn")]
    UnauthorizedPayToSpawn,

    #[msg("Player not found")]
    PlayerNotFound,

    #[msg("Invalid player token account")]
    InvalidPlayerTokenAccount,

    #[msg("Invalid player")]
    InvalidPlayer,

    #[msg("Player has no spawns")]
    PlayerHasNoSpawns,

    #[msg("Game is not in progress")]
    GameNotInProgress,

    #[msg("Invalid session ID - must be 12-32 chars, alphanumeric with underscores/hyphens")]
    InvalidSessionId,

    #[msg("Vault has insufficient funds for operation")]
    InsufficientVaultFunds,

    #[msg("Game session already completed - no further operations allowed")]
    AlreadyCompleted,

    #[msg("Cross-team kills only - cannot kill teammates")]
    InvalidKillTarget,

    #[msg("Spawn purchase limit exceeded - maximum 100 spawns per player")]
    SpawnLimitExceeded,

    #[msg("Game session expired - operations not allowed after timeout")]
    SessionExpired,

    #[msg("Emergency pause is active - all operations suspended")]
    EmergencyPaused,

    #[msg("Invalid token program provided")]
    InvalidTokenProgram,

    #[msg("Account substitution detected - provided account does not match expected")]
    AccountSubstitution,

    #[msg("Duplicate player detected - player already exists in game session")]
    DuplicatePlayer,

    #[msg("Game timeout exceeded - maximum 24 hours")]
    GameTimeout,

    #[msg("Invalid bet amount - must be between 0.001 and 1,000 tokens")]
    InvalidBetAmount,

    #[msg("Invalid timestamp - negative or zero timestamps not allowed")]
    InvalidTimestamp,

    #[msg("Concurrent operation detected - please retry")]
    ConcurrentOperation,

    #[msg("Rate limit exceeded - too many operations in short time")]
    RateLimitExceeded,

    #[msg("Invalid account derivation - PDA seeds do not match")]
    InvalidAccountDerivation,

    #[msg("Account initialization failed - insufficient rent or invalid parameters")]
    AccountInitializationFailed,

    #[msg("Token transfer failed - insufficient balance or invalid accounts")]
    TokenTransferFailed,

    #[msg("Invalid game mode for operation")]
    InvalidGameModeForOperation,

    #[msg("Player index out of bounds for game mode")]
    PlayerIndexOutOfBounds,

    #[msg("Invalid kill count - exceeds maximum allowed")]
    InvalidKillCount,

    #[msg("Session ID collision detected - use different session ID")]
    SessionIdCollision,

    #[msg("Authority mismatch - signer does not match game authority")]
    AuthorityMismatch,

    #[msg("Invalid buffer size - account space insufficient")]
    InvalidBufferSize,

    #[msg("Vault balance mismatch - expected balance does not match actual")]
    VaultBalanceMismatch,

    #[msg("Invalid spawn increment - must be between 1 and 50")]
    InvalidSpawnIncrement,

    #[msg("Team composition invalid - incorrect number of players")]
    InvalidTeamComposition,

    #[msg("Game data corruption detected")]
    GameDataCorruption,

    #[msg("Invalid account ownership - account owner mismatch")]
    InvalidAccountOwnership,

    #[msg("Insufficient entropy in session ID - must be more random")]
    InsufficientEntropy,

    #[msg("Stake amount exceeds maximum allowed per player")]
    StakeAmountExceeded,

    #[msg("Minimum stake requirement not met")]
    MinimumStakeNotMet,

    #[msg("Invalid operation sequence - operations must be performed in correct order")]
    InvalidOperationSequence,

    #[msg("Account freeze detected - operation not allowed")]
    AccountFrozen,

    #[msg("Invalid signature - transaction not properly signed")]
    InvalidSignature,

    #[msg("Replay attack detected - nonce already used")]
    ReplayAttack,

    #[msg("Invalid program version - upgrade required")]
    InvalidProgramVersion,

    #[msg("Circuit breaker activated - system protection engaged")]
    CircuitBreakerActivated,

    #[msg("Value too large - exceeds maximum safe limits")]
    ValueTooLarge,
}

impl WagerError {
    /// Get error severity level for logging and monitoring
    pub fn severity(&self) -> ErrorSeverity {
        match self {
            // Critical security errors
            Self::GameDataCorruption
            | Self::ReplayAttack
            | Self::AccountSubstitution
            | Self::InvalidSignature => ErrorSeverity::Critical,

            // High priority errors
            Self::ConcurrentOperation
            | Self::RateLimitExceeded
            | Self::ArithmeticError
            | Self::InvalidBetAmount
            | Self::SessionIdCollision => ErrorSeverity::High,

            // Medium priority errors
            Self::InvalidGameState
            | Self::DuplicatePlayer
            | Self::InvalidKillCount
            | Self::SpawnLimitExceeded => ErrorSeverity::Medium,

            // Low priority errors
            Self::PlayerNotFound | Self::TeamIsFull | Self::InvalidPlayer => ErrorSeverity::Low,

            // Default to medium for unlisted errors
            _ => ErrorSeverity::Medium,
        }
    }

    /// Check if error should trigger emergency pause
    pub fn should_emergency_pause(&self) -> bool {
        matches!(
            self,
            Self::GameDataCorruption
                | Self::ReplayAttack
                | Self::AccountSubstitution
                | Self::CircuitBreakerActivated
        )
    }

    /// Check if error is retryable
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::ConcurrentOperation
                | Self::RateLimitExceeded
                | Self::InsufficientVaultFunds
                | Self::InvalidGameState // Might be retryable depending on context
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ErrorSeverity {
    Critical,
    High,
    Medium,
    Low,
}

impl ErrorSeverity {
    pub fn to_string(&self) -> &'static str {
        match self {
            Self::Critical => "CRITICAL",
            Self::High => "HIGH",
            Self::Medium => "MEDIUM",
            Self::Low => "LOW",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_severity_classification() {
        assert_eq!(
            WagerError::GameDataCorruption.severity(),
            ErrorSeverity::Critical
        );
        assert_eq!(WagerError::ArithmeticError.severity(), ErrorSeverity::High);
        assert_eq!(WagerError::PlayerNotFound.severity(), ErrorSeverity::Low);
    }

    #[test]
    fn test_emergency_pause_triggers() {
        assert!(WagerError::GameDataCorruption.should_emergency_pause());
        assert!(WagerError::ReplayAttack.should_emergency_pause());
        assert!(!WagerError::PlayerNotFound.should_emergency_pause());
    }

    #[test]
    fn test_retryable_errors() {
        assert!(WagerError::ConcurrentOperation.is_retryable());
        assert!(WagerError::RateLimitExceeded.is_retryable());
        assert!(!WagerError::GameDataCorruption.is_retryable());
    }
}
