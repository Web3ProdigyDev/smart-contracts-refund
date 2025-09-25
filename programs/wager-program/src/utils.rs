// utils.rs - ENHANCED SESSION ID VALIDATION AND SHARED FUNCTIONS (SECURITY HARDENED)
use crate::errors::WagerError;
use anchor_lang::prelude::*;
use sha2::{Digest, Sha256};

/// CRITICAL FIX: Enhanced session ID validation with collision-resistant patterns
pub fn validate_session_id(session_id: &str) -> Result<()> {
    // Length validation - must be reasonable size with minimum entropy
    require!(
        session_id.len() >= 12 && session_id.len() <= 32,
        WagerError::InvalidSessionId
    );

    // Character validation - only alphanumeric and safe separators
    require!(
        session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        WagerError::InvalidSessionId
    );

    // Must contain at least one letter and one number for entropy
    let has_letter = session_id.chars().any(|c| c.is_ascii_alphabetic());
    let has_number = session_id.chars().any(|c| c.is_ascii_digit());
    require!(has_letter && has_number, WagerError::InvalidSessionId);

    // Prevent null bytes and control characters explicitly
    require!(
        !session_id.contains('\0') && !session_id.chars().any(|c| c.is_control()),
        WagerError::InvalidSessionId
    );

    // Prevent common problematic patterns that could cause PDA collisions
    let lower_id = session_id.to_lowercase();
    require!(
        !lower_id.starts_with("000") && // Prevent leading zeros
        !lower_id.starts_with("111") && // Prevent repeated patterns
        !lower_id.starts_with("aaa") && // Prevent repeated patterns
        !lower_id.eq("default") &&
        !lower_id.eq("admin") &&
        !lower_id.eq("system") &&
        !lower_id.eq("test") &&
        !lower_id.eq("game") &&
        !lower_id.contains("..") && // Prevent double separators
        !lower_id.contains("__") && // Prevent double underscores
        !lower_id.contains("--"), // Prevent double dashes
        WagerError::InvalidSessionId
    );

    // Ensure session ID doesn't end with separators
    require!(
        !session_id.ends_with('_') && !session_id.ends_with('-'),
        WagerError::InvalidSessionId
    );

    // Additional entropy check - prevent too many repeated characters
    use std::collections::HashMap;
    use std::collections::HashSet;

    let mut char_counts = std::collections::HashMap::new();
    for ch in session_id.chars() {
        *char_counts.entry(ch).or_insert(0) += 1;
    }

    // No single character should appear more than 40% of the time
    let max_allowed_repeats = (session_id.len() * 4) / 10;
    require!(
        char_counts
            .values()
            .all(|&count| count <= max_allowed_repeats),
        WagerError::InvalidSessionId
    );

    Ok(())
}

/// CRITICAL FIX: Enhanced remaining accounts validation with comprehensive checks
pub fn validate_remaining_accounts_against_players(
    remaining_accounts: &[AccountInfo],
    expected_players: &[Pubkey],
) -> Result<()> {
    // Check we have exactly the right number of accounts (player + token account pairs)
    require!(
        remaining_accounts.len() == expected_players.len() * 2,
        WagerError::InvalidRemainingAccounts
    );

    // Ensure we don't have empty player list
    require!(
        !expected_players.is_empty(),
        WagerError::InvalidRemainingAccounts
    );

    // Validate no duplicate accounts provided
    let mut seen_accounts = std::collections::HashSet::new();
    for account in remaining_accounts {
        require!(
            seen_accounts.insert(account.key()),
            WagerError::DuplicatePlayer
        );
    }

    // Validate no default pubkeys in expected players
    for expected_player in expected_players {
        require!(
            *expected_player != Pubkey::default(),
            WagerError::InvalidPlayer
        );
    }

    // Validate each player account is present and in correct position
    for (i, expected_player) in expected_players.iter().enumerate() {
        let player_account_idx = i * 2;
        let token_account_idx = i * 2 + 1;

        // Bounds checking
        require!(
            player_account_idx < remaining_accounts.len()
                && token_account_idx < remaining_accounts.len(),
            WagerError::InvalidRemainingAccounts
        );

        let player_account = &remaining_accounts[player_account_idx];
        let token_account = &remaining_accounts[token_account_idx];

        require!(
            player_account.key() == *expected_player,
            WagerError::InvalidPlayer
        );

        // Validate token account is different from player account
        require!(
            token_account.key() != player_account.key(),
            WagerError::InvalidPlayerTokenAccount
        );

        // Validate accounts are not system accounts
        require!(
            player_account.key() != anchor_lang::system_program::ID
                && token_account.key() != anchor_lang::system_program::ID,
            WagerError::InvalidPlayer
        );
    }

    Ok(())
}

/// ENHANCED: Comprehensive bounds checking for arithmetic operations with minimum values
pub fn safe_multiply_and_divide(base: u64, multiplier: u64, divisor: u64) -> Result<u64> {
    require!(divisor > 0, WagerError::ArithmeticError);
    require!(base > 0, WagerError::ArithmeticError);
    require!(multiplier > 0, WagerError::ArithmeticError);

    // Check for overflow in multiplication using division check
    require!(base <= u64::MAX / multiplier, WagerError::ArithmeticError);

    let product = base
        .checked_mul(multiplier)
        .ok_or(WagerError::ArithmeticError)?;
    let result = product
        .checked_div(divisor)
        .ok_or(WagerError::ArithmeticError)?;

    // Ensure result is reasonable (not zero unless inputs are very small)
    if base >= divisor && multiplier >= divisor {
        require!(result > 0, WagerError::ArithmeticError);
    }

    Ok(result)
}

/// ENHANCED: Safe addition with overflow protection
pub fn safe_add_u64(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b).ok_or(error!(WagerError::ArithmeticError))
}

/// ENHANCED: Safe subtraction with underflow protection  
pub fn safe_sub_u64(a: u64, b: u64) -> Result<u64> {
    a.checked_sub(b).ok_or(error!(WagerError::ArithmeticError))
}

/// ENHANCED: Safe multiplication with overflow protection
pub fn safe_mul_u64(a: u64, b: u64) -> Result<u64> {
    a.checked_mul(b).ok_or(error!(WagerError::ArithmeticError))
}

/// CRITICAL FIX: Enhanced vault balance validation with minimum buffer amounts
pub fn validate_vault_balance(vault_balance: u64, required_amount: u64) -> Result<()> {
    require!(
        vault_balance >= required_amount,
        WagerError::InsufficientVaultFunds
    );

    // FIXED: Ensure minimum safety margin of at least 1000 lamports (0.001 SOL equivalent)
    let min_safety_margin = 1000u64;
    let percentage_margin = required_amount / 1000; // 0.1% margin
    let safety_margin = std::cmp::max(min_safety_margin, percentage_margin);

    require!(
        vault_balance >= required_amount.saturating_add(safety_margin),
        WagerError::InsufficientVaultFunds
    );

    Ok(())
}

/// NEW: Validate bet amount is within reasonable bounds
pub fn validate_bet_amount(bet_amount: u64) -> Result<()> {
    // Use constants from state.rs instead
    require!(
        bet_amount >= crate::state::MIN_BET,
        WagerError::InvalidBetAmount
    );

    require!(
        bet_amount <= crate::state::MAX_BET,
        WagerError::InvalidBetAmount
    );

    Ok(())
}

/// NEW: Validate spawn counts are within reasonable limits
pub fn validate_spawn_count(current_spawns: u8, increment: u8) -> Result<()> {
    const MAX_SPAWNS: u8 = 100;

    require!(
        current_spawns <= MAX_SPAWNS.saturating_sub(increment),
        WagerError::SpawnLimitExceeded
    );

    // Additional check for reasonable increment
    require!(
        increment > 0 && increment <= 50, // Max 50 spawns per purchase
        WagerError::InvalidSpawns
    );

    // Check for overflow
    require!(
        current_spawns.checked_add(increment).is_some(),
        WagerError::ArithmeticError
    );

    Ok(())
}

/// NEW: Validate kill counts are within reasonable limits  
pub fn validate_kill_count(current_kills: u8) -> Result<()> {
    const MAX_KILLS: u8 = 200; // Reasonable maximum

    require!(current_kills < MAX_KILLS, WagerError::ArithmeticError);

    Ok(())
}

/// CRITICAL SECURITY FIX: Generate cryptographically secure session validation hash
/// Replaced DefaultHasher with SHA-256 for collision resistance
pub fn generate_session_hash(session_id: &str, authority: Pubkey, timestamp: i64) -> [u8; 32] {
    let mut hasher = Sha256::new();

    // Include multiple entropy sources with secure hashing
    hasher.update(b"WAGER_SESSION_V2_"); // Version prefix
    hasher.update(session_id.as_bytes());
    hasher.update(&authority.to_bytes());
    hasher.update(&timestamp.to_le_bytes());

    // Add additional entropy from clock if available
    if let Ok(clock) = Clock::get() {
        hasher.update(&clock.slot.to_le_bytes());
        hasher.update(&clock.epoch.to_le_bytes());
    }

    // Add program ID for domain separation
    hasher.update(&crate::ID.to_bytes());

    // Finalize with cryptographic hash
    hasher.finalize().into()
}

/// NEW: Verify session hash integrity
pub fn verify_session_hash(
    stored_hash: &[u8; 32],
    session_id: &str,
    authority: Pubkey,
    timestamp: i64,
) -> Result<()> {
    let computed_hash = generate_session_hash(session_id, authority, timestamp);

    require!(
        stored_hash == &computed_hash,
        WagerError::SessionIdCollision
    );

    Ok(())
}

/// NEW: Generate cryptographically secure nonce
pub fn generate_secure_nonce(base_nonce: u64, additional_entropy: &[u8]) -> u64 {
    let mut hasher = Sha256::new();

    hasher.update(&base_nonce.to_le_bytes());
    hasher.update(additional_entropy);

    if let Ok(clock) = Clock::get() {
        hasher.update(&clock.unix_timestamp.to_le_bytes());
        hasher.update(&clock.slot.to_le_bytes());
    }

    let hash_result = hasher.finalize();

    // Use first 8 bytes of hash as secure nonce
    u64::from_le_bytes([
        hash_result[0],
        hash_result[1],
        hash_result[2],
        hash_result[3],
        hash_result[4],
        hash_result[5],
        hash_result[6],
        hash_result[7],
    ])
}
