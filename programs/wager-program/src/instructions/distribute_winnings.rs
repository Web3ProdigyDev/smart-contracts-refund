// distribute_winnings.rs - ATOMIC OPERATIONS WITH RACE CONDITION PREVENTION
use crate::{errors::WagerError, state::*, TOKEN_ID, utils::{validate_session_id, validate_remaining_accounts_against_players, safe_multiply_and_divide}};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn distribute_pay_spawn_earnings<'info>(
    ctx: Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    session_id: String,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let vault_bump = game_session.vault_bump;
    
    // Validate session_id format FIRST
    validate_session_id(&session_id)?;
    
    // CRITICAL RACE CONDITION FIX: Atomic status transition with nonce validation
    let current_nonce = game_session.nonce;
    
    // ATOMIC OPERATION: Check current state and transition atomically
    require!(
        game_session.status == GameStatus::InProgress,
        WagerError::InvalidGameState
    );
    
    // IMMEDIATE atomic transition to prevent race conditions
    game_session.atomic_status_transition(GameStatus::Completed, current_nonce)?;
    
    // Verify atomic transition succeeded
    require!(
        game_session.status == GameStatus::Completed,
        WagerError::InvalidGameState
    );
    
    require!(
        game_session.nonce == current_nonce + 1,
        WagerError::ConcurrentOperation
    );
    
    msg!("Distribution starting for session: {} (status atomically updated, nonce: {})", 
         session_id, game_session.nonce);

    // ENHANCED: Comprehensive validation before any transfers
    game_session.validate_integrity()?;
    
    let players = game_session.get_all_players();
    let active_players: Vec<Pubkey> = players
        .into_iter()
        .filter(|p| *p != Pubkey::default())
        .collect();
        
    msg!("Number of active players: {}", active_players.len());
    msg!("Number of remaining accounts: {}", ctx.remaining_accounts.len());

    // Enhanced validation for remaining accounts
    require!(
        !ctx.remaining_accounts.is_empty(),
        WagerError::InvalidRemainingAccounts
    );

    require!(
        ctx.remaining_accounts.len() % 2 == 0,
        WagerError::InvalidRemainingAccounts
    );

    // Pre-validate vault balance before any calculations
    let initial_vault_balance = ctx.accounts.vault_token_account.amount;
    require!(initial_vault_balance > 0, WagerError::InsufficientVaultFunds);
    
    // CRITICAL FIX: Calculate total required funds with comprehensive validation
    let mut total_required: u64 = 0;
    let mut player_earnings: Vec<(Pubkey, u64)> = Vec::new();
    
    for player in active_players.iter().cloned() {
        if player == Pubkey::default() {
            continue;
        }
        
        let kills_and_spawns = game_session.get_kills_and_spawns(player)?;
        if kills_and_spawns == 0 {
            continue;
        }

        // CRITICAL FIX: Enhanced arithmetic with comprehensive bounds checking
        let kills_and_spawns_u64 = kills_and_spawns as u64;
        
        // Prevent manipulation by capping earnings to reasonable amount
        const MAX_EARNINGS_MULTIPLIER: u64 = 50;
        require!(
            kills_and_spawns_u64 <= MAX_EARNINGS_MULTIPLIER,
            WagerError::ArithmeticError
        );
        
        // Use safe utility function for multiplication and division
        let earnings = safe_multiply_and_divide(
            kills_and_spawns_u64,
            game_session.session_bet,
            10
        )?;
        
        // Additional validation: earnings should be reasonable relative to bet
        let max_reasonable_earnings = game_session.session_bet
            .checked_mul(MAX_EARNINGS_MULTIPLIER)
            .ok_or(WagerError::ArithmeticError)?;
        
        require!(
            earnings <= max_reasonable_earnings,
            WagerError::ArithmeticError
        );
        
        // CRITICAL FIX: Safe total accumulation with overflow protection
        require!(
            total_required <= u64::MAX - earnings,
            WagerError::ArithmeticError
        );
            
        total_required = total_required
            .checked_add(earnings)
            .ok_or(WagerError::ArithmeticError)?;
            
        player_earnings.push((player, earnings));
    }
    
    // CRITICAL FIX: Enhanced vault balance validation with fixed safety buffer
    require!(
        initial_vault_balance >= total_required,
        WagerError::InsufficientVaultFunds
    );
    
    // FIXED: Minimum safety buffer of 1000 tokens instead of percentage
    let min_safety_buffer = 1000u64;
    let percentage_buffer = total_required / 1000; // 0.1%
    let safety_buffer = std::cmp::max(min_safety_buffer, percentage_buffer);
    
    require!(
        initial_vault_balance >= total_required.saturating_add(safety_buffer),
        WagerError::InsufficientVaultFunds
    );
    
    msg!("Total required: {}, Vault balance: {}, Safety buffer: {}", 
         total_required, initial_vault_balance, safety_buffer);

    // Validate remaining accounts against earning players only
    let earning_players: Vec<Pubkey> = player_earnings.iter().map(|(pubkey, _)| *pubkey).collect();
    validate_remaining_accounts_against_players(&ctx.remaining_accounts, &earning_players)?;

    // Track actual distributed amount for final validation
    let mut total_distributed: u64 = 0;
    let mut successful_distributions: usize = 0;

    // Perform distributions with comprehensive validation and rollback capability
    for (player, earnings) in player_earnings {
        if earnings == 0 {
            continue;
        }

        // Find player account with strict validation
        let player_index = ctx
            .remaining_accounts
            .iter()
            .step_by(2)
            .position(|acc| acc.key() == player)
            .ok_or(WagerError::InvalidPlayer)?;

        let player_account = &ctx.remaining_accounts[player_index * 2];
        let player_token_account_info = &ctx.remaining_accounts[player_index * 2 + 1];
        let player_token_account = Account::<TokenAccount>::try_from(player_token_account_info)?;

        // CRITICAL FIX: Comprehensive token account validation
        require!(
            player_token_account.owner == player_account.key(),
            WagerError::InvalidPlayerTokenAccount
        );

        require!(
            player_token_account.mint == TOKEN_ID,
            WagerError::InvalidTokenMint
        );

        require!(
            player_account.key() == player,
            WagerError::InvalidPlayer
        );

        msg!("Transferring {} to player {} (index {})", earnings, player, player_index);

        // Enhanced transfer with better error handling
        let transfer_result = anchor_spl::token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                anchor_spl::token::Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: player_token_account_info.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[&[
                    b"vault",
                    session_id.as_bytes(),
                    ctx.accounts.game_server.key().as_ref(),
                    &[vault_bump],
                ]],
            ),
            earnings,
        );
        
        // Handle individual transfer failures
        if let Err(e) = transfer_result {
            msg!("Transfer failed for player {}: {:?}", player, e);
            // Could implement partial refund logic here if needed
            return Err(e.into());
        }
        
        // Track distributed amount
        total_distributed = total_distributed
            .checked_add(earnings)
            .ok_or(WagerError::ArithmeticError)?;
        
        successful_distributions += 1;
    }
    
    // Final comprehensive validation
    require!(
        total_distributed == total_required,
        WagerError::IncompleteDistribution
    );
    
    require!(
        successful_distributions == earning_players.len(),
        WagerError::IncompleteDistribution
    );
    
    msg!("Distribution completed successfully:");
    msg!("  Total distributed: {} tokens", total_distributed);
    msg!("  Successful distributions: {}", successful_distributions);
    msg!("  Final nonce: {}", game_session.nonce);

    Ok(())
}

pub fn distribute_all_winnings_handler<'info>(
    ctx: Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    session_id: String,
    winning_team: u8,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    
    // Validate session_id format
    validate_session_id(&session_id)?;
    
    msg!("Starting winner-take-all distribution for session: {}", session_id);

    // Verify authority
    require!(
        game_session.authority == ctx.accounts.game_server.key(),
        WagerError::UnauthorizedDistribution
    );
    
    // CRITICAL RACE CONDITION FIX: Atomic status transition with nonce tracking
    let current_nonce = game_session.nonce;
    
    require!(
        game_session.status == GameStatus::InProgress,
        WagerError::InvalidGameState
    );
    
    // IMMEDIATE atomic transition
    game_session.atomic_status_transition(GameStatus::Completed, current_nonce)?;
    
    // Verify atomic transition
    require!(
        game_session.status == GameStatus::Completed && game_session.nonce == current_nonce + 1,
        WagerError::ConcurrentOperation
    );

    // Validate winning team selection
    require!(
        winning_team == 0 || winning_team == 1,
        WagerError::InvalidWinningTeam
    );

    let players_per_team = game_session.game_mode.players_per_team();
    let vault_bump = game_session.vault_bump;

    // CRITICAL FIX: Enhanced team validation
    require!(
        game_session.check_all_filled_secure()?,
        WagerError::NotAllPlayersJoined
    );

    // Get the winning team
    let winning_players = if winning_team == 0 {
        &game_session.team_a.players[0..players_per_team]
    } else {
        &game_session.team_b.players[0..players_per_team]
    };

    // Validate no default/empty players in winning team
    for player in winning_players {
        require!(
            *player != Pubkey::default(),
            WagerError::InvalidPlayer
        );
    }

    require!(
        ctx.remaining_accounts.len() >= 2 * players_per_team,
        WagerError::InvalidRemainingAccounts
    );

    // CRITICAL FIX: Enhanced arithmetic validation for payouts
    let session_bet = game_session.session_bet;
    let players_per_team_u64 = players_per_team as u64;
    
    let winning_amount = session_bet
        .checked_mul(2)
        .ok_or(WagerError::ArithmeticError)?;
    
    let total_payout = winning_amount
        .checked_mul(players_per_team_u64)
        .ok_or(WagerError::ArithmeticError)?;
        
    // Enhanced vault validation with fixed safety buffer
    let vault_balance = ctx.accounts.vault_token_account.amount;
    require!(
        vault_balance >= total_payout,
        WagerError::InsufficientVaultFunds
    );
    
    let min_safety_buffer = 1000u64;
    let percentage_buffer = total_payout / 1000;
    let safety_buffer = std::cmp::max(min_safety_buffer, percentage_buffer);
    
    require!(
        vault_balance >= total_payout.saturating_add(safety_buffer),
        WagerError::InsufficientVaultFunds
    );

    // Validate remaining accounts match winning players exactly
    for i in 0..players_per_team {
        let expected_winner = winning_players[i];
        let provided_account = &ctx.remaining_accounts[i * 2];
        
        require!(
            provided_account.key() == expected_winner,
            WagerError::InvalidWinner
        );
    }

    let mut total_distributed: u64 = 0;

    for i in 0..players_per_team {
        let winner = &ctx.remaining_accounts[i * 2];
        let winner_token_account_info = &ctx.remaining_accounts[i * 2 + 1];
        let winner_token_account = Account::<TokenAccount>::try_from(winner_token_account_info)?;

        // Enhanced winner validation
        require!(
            winner_token_account.owner == winner.key(),
            WagerError::InvalidWinnerTokenAccount
        );

        require!(
            winner_token_account.mint == TOKEN_ID,
            WagerError::InvalidTokenMint
        );

        let winner_pubkey = winner.key();
        require!(
            winning_players.iter().any(|&p| p == winner_pubkey),
            WagerError::InvalidWinner
        );

        msg!("Transferring {} to winner {} (position {})", winning_amount, winner_pubkey, i);

        // Transfer tokens from vault to winner
        anchor_spl::token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                anchor_spl::token::Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: winner_token_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[&[
                    b"vault",
                    session_id.as_bytes(),
                    ctx.accounts.game_server.key().as_ref(),
                    &[vault_bump],
                ]],
            ),
            winning_amount,
        )?;
        
        total_distributed = total_distributed
            .checked_add(winning_amount)
            .ok_or(WagerError::ArithmeticError)?;
    }
    
    // Final validation
    require!(
        total_distributed == total_payout,
        WagerError::IncompleteDistribution
    );
    
    msg!("Winner-take-all distribution completed:");
    msg!("  Total distributed: {} tokens", total_distributed);
    msg!("  Winners on team: {}", winning_team);
    msg!("  Players per team: {}", players_per_team);
    msg!("  Final nonce: {}", game_session.nonce);

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct DistributeWinnings<'info> {
    /// The game server authority that created the session
    #[account(
        constraint = game_server.is_signer @ WagerError::UnauthorizedDistribution,
    )]
    pub game_server: Signer<'info>,

    // ENHANCED: PDA with comprehensive validation
    #[account(
        mut,
        seeds = [
            b"game_session", 
            session_id.as_bytes(),
            game_server.key().as_ref()
        ],
        bump = game_session.bump,
        constraint = game_session.authority == game_server.key() @ WagerError::UnauthorizedDistribution,
        constraint = game_session.session_id == session_id @ WagerError::InvalidSessionId,
    )]
    pub game_session: Account<'info, GameSession>,

    /// CHECK: Vault PDA with enhanced validation
    #[account(
        mut,
        seeds = [
            b"vault", 
            session_id.as_bytes(),
            game_server.key().as_ref()
        ],
        bump = game_session.vault_bump,
    )]
    pub vault: AccountInfo<'info>,

    #[account(
        mut,
        associated_token::mint = TOKEN_ID,
        associated_token::authority = vault,
        constraint = vault_token_account.mint == TOKEN_ID @ WagerError::InvalidTokenMint,
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    #[account(
        constraint = token_program.key() == anchor_spl::token::ID @ WagerError::InvalidTokenProgram,
    )]
    pub token_program: Program<'info, Token>,
    
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}