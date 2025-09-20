// refund_wager.rs - SECURITY HARDENED VERSION
use crate::{errors::WagerError, state::*, TOKEN_ID, utils::{validate_session_id, validate_remaining_accounts_against_players}};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn refund_wager_handler<'info>(
    ctx: Context<'_, '_, 'info, 'info, RefundWager<'info>>,
    session_id: String,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let vault_bump = game_session.vault_bump; // Store vault_bump early
    
    // Validate session_id format
    validate_session_id(&session_id)?;
    
    msg!("Starting Refund for session: {}", session_id);

    // CRITICAL FIX: Only allow refunds for games that are not completed
    // and ensure we don't double-refund
    require!(
        game_session.status != GameStatus::Completed,
        WagerError::InvalidGameState
    );
    
    // Mark as completed FIRST to prevent double refunds
    game_session.status = GameStatus::Completed;

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

    // CRITICAL FIX: Calculate total refund needed and verify vault balance
    let refund_amount = game_session.session_bet;
    let total_refund = refund_amount
        .checked_mul(active_players.len() as u64)
        .ok_or(WagerError::ArithmeticError)?;
        
    require!(
        ctx.accounts.vault_token_account.amount >= total_refund,
        WagerError::InsufficientVaultFunds
    );
    
    msg!("Total refund required: {}, Vault balance: {}", total_refund, ctx.accounts.vault_token_account.amount);

    // CRITICAL FIX: Validate remaining accounts match active players
    validate_remaining_accounts_against_players(&ctx.remaining_accounts, &active_players)?;

    for player in active_players {
        msg!("Processing refund for player: {}", player);

        // CRITICAL FIX: Strict player account lookup
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

        // CRITICAL FIX: Verify account identity
        require!(
            player_account.key() == player,
            WagerError::InvalidPlayer
        );

        msg!("Refunding {} to player {}", refund_amount, player);

        // Transfer tokens from vault to player
        anchor_spl::token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                anchor_spl::token::Transfer {
                    from: ctx.accounts.vault_token_account.to_account_info(),
                    to: player_token_account.to_account_info(),
                    authority: ctx.accounts.vault.to_account_info(),
                },
                &[&[
                    b"vault",
                    session_id.as_bytes(),
                    &[vault_bump],
                ]],
            ),
            refund_amount,
        )?;
    }

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct RefundWager<'info> {
    /// The game server authority that created the session
    pub game_server: Signer<'info>,

    #[account(
        mut,
        seeds = [b"game_session", session_id.as_bytes()],
        bump = game_session.bump,
        constraint = game_session.authority == game_server.key() @ WagerError::UnauthorizedDistribution,
    )]
    pub game_session: Account<'info, GameSession>,

    /// CHECK: Vault PDA that holds the funds
    #[account(
        mut,
        seeds = [b"vault", session_id.as_bytes()],
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