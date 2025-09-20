// utils.rs - SHARED VALIDATION FUNCTIONS
use anchor_lang::prelude::*;
use crate::errors::WagerError;

/// CRITICAL FIX: Session ID validation function (shared across modules)
pub fn validate_session_id(session_id: &str) -> Result<()> {
    require!(
        !session_id.is_empty() && session_id.len() <= 32,
        WagerError::InvalidSessionId
    );
    
    // Prevent control characters and null bytes that could cause PDA collisions
    require!(
        session_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        WagerError::InvalidSessionId
    );
    
    // Prevent null bytes specifically
    require!(
        !session_id.contains('\0'),
        WagerError::InvalidSessionId
    );
    
    Ok(())
}

/// CRITICAL FIX: Strict remaining accounts validation (shared across modules)
pub fn validate_remaining_accounts_against_players(
    remaining_accounts: &[AccountInfo],
    expected_players: &[Pubkey]
) -> Result<()> {
    // Check we have exactly the right number of accounts (player + token account pairs)
    require!(
        remaining_accounts.len() == expected_players.len() * 2,
        WagerError::InvalidRemainingAccounts
    );
    
    // Validate each player account is present and in correct position
    for (i, expected_player) in expected_players.iter().enumerate() {
        let player_account = &remaining_accounts[i * 2];
        require!(
            player_account.key() == *expected_player,
            WagerError::InvalidPlayer
        );
    }
    
    Ok(())
}