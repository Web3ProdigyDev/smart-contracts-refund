// refund_wager.rs - ATOMIC OPERATIONS SECURITY HARDENED VERSION
use crate::{errors::WagerError, state::*, TOKEN_ID, utils::{validate_session_id, validate_remaining_accounts_against_players, safe_add_u64}};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn refund_wager_handler<'info>(
    ctx: Context<'_, '_, 'info, 'info, RefundWager<'info>>,
    session_id: String,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let vault_bump = game_session.vault_bump;
    
    // Validate session_id format
    validate_session_id(&session_id)?;
    
    msg!("Starting Refund for session: {}", session_id);

    // CRITICAL FIX: Atomic status validation and update
    // Only allow refunds for games that are not completed and ensure no double-refunds
    require!(
        game_session.status != GameStatus::Completed,
        WagerError::InvalidGameState
    );
    
    // ATOMIC OPERATION: Mark as completed IMMEDIATELY to prevent double refunds
    let original_status = game_session.status.clone();
    game_session.status = GameStatus::Completed;
    
    // Verify atomic update succeeded
    require!(
        game_session.status == GameStatus::Completed,
        WagerError::InvalidGameState
    );
    
    msg!("Game status atomically changed from {:?} to Completed", original_status);

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

    require!(
        ctx.remaining_accounts.len() == active_players.len() * 2,
        WagerError::InvalidRemainingAccounts
    );

    // CRITICAL FIX: Enhanced refund calculation with comprehensive validation
    let refund_amount = game_session.session_bet;
    
    // Validate refund amount is reasonable
    require!(refund_amount > 0, WagerError::InvalidBetAmount);
    require!(refund_amount <= 100_000_000_000, WagerError::InvalidBetAmount); // Max 100K tokens
    
    // Calculate total with safe arithmetic
    let mut total_refund: u64 = 0;
    for _ in 0..active_players.len() {
        total_refund = safe_add_u64(total_refund, refund_amount)?;
    }
    
    // Pre-validate vault has sufficient balance with safety buffer
    let vault_balance = ctx.accounts.vault_token_account.amount;
    require!(
        vault_balance >= total_refund,
        WagerError::InsufficientVaultFunds
    );
    
    // Additional safety buffer (1% extra) to handle any edge cases
    let safety_buffer = total_refund / 100;
    require!(
        vault_balance >= total_refund.saturating_add(safety_buffer),
        WagerError::InsufficientVaultFunds
    );
    
    msg!("Total refund required: {}, Vault balance: {}, Safety buffer: {}", 
         total_refund, vault_balance, safety_buffer);

    // CRITICAL FIX: Validate remaining accounts match active players exactly
    validate_remaining_accounts_against_players(&ctx.remaining_accounts, &active_players)?;

    // Track actual refunded amount for validation
    let mut total_refunded: u64 = 0;
    let mut successful_refunds: usize = 0;

    for (player_idx, player) in active_players.iter().enumerate() {
        msg!("Processing refund for player: {} (index: {})", player, player_idx);

        // CRITICAL FIX: Strict player account lookup with bounds checking
        let player_account_idx = player_idx * 2;
        let token_account_idx = player_idx * 2 + 1;
        
        require!(
            player_account_idx < ctx.remaining_accounts.len(),
            WagerError::InvalidRemainingAccounts
        );
        require!(
            token_account_idx < ctx.remaining_accounts.len(),
            WagerError::InvalidRemainingAccounts
        );

        let player_account = &ctx.remaining_accounts[player_account_idx];
        let player_token_account_info = &ctx.remaining_accounts[token_account_idx];
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
            player_account.key() == *player,
            WagerError::InvalidPlayer
        );

        msg!("Refunding {} to player {} via token account {}", 
             refund_amount, player, player_token_account_info.key());

        // Transfer tokens from vault to player with comprehensive error handling
        anchor_spl::token::transfer(
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
                    ctx.accounts.game_server.key().as_ref(), // Updated seed with authority
                    &[vault_bump],
                ]],
            ),
            refund_amount,
        )?;
        
        // Track successful refund
        total_refunded = safe_add_u64(total_refunded, refund_amount)?;
        successful_refunds += 1;
        
        msg!("Successfully refunded {} tokens to player {} (total refunded so far: {})", 
             refund_amount, player, total_refunded);
    }

    // CRITICAL FIX: Final validation ensures all refunds completed successfully
    require!(
        total_refunded == total_refund,
        WagerError::IncompleteDistribution
    );
    
    require!(
        successful_refunds == active_players.len(),
        WagerError::IncompleteDistribution
    );
    
    msg!("Refund completed successfully: {} tokens refunded to {} players", 
         total_refunded, successful_refunds);

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct RefundWager<'info> {
    /// The game server authority that created the session
    pub game_server: Signer<'info>,

    // UPDATED: Enhanced PDA with authority in seeds
    #[account(
        mut,
        seeds = [
            b"game_session", 
            session_id.as_bytes(),
            game_server.key().as_ref()
        ],
        bump = game_session.bump,
        constraint = game_session.authority == game_server.key() @ WagerError::UnauthorizedDistribution,
    )]
    pub game_session: Account<'info, GameSession>,

    /// CHECK: Vault PDA derived from session_id and authority - used only for token transfers
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
        associated_token::authority = vault
    )]
    pub vault_token_account: Account<'info, TokenAccount>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}