use crate::{
    errors::WagerError,
    state::*,
    utils::{safe_add_u64, validate_remaining_accounts_against_players, validate_session_id},
    TOKEN_ID,
};
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

    msg!("Starting refund for session: {}", session_id);

    // Atomic refund initiation
    let current_nonce = game_session.nonce;
    let success = match game_session.status {
        GameStatus::WaitingForPlayers => game_session.compare_and_swap_status(
            GameStatus::WaitingForPlayers,
            current_nonce,
            GameStatus::RefundInProgress,
            Some("refund_from_waiting"),
        )?,
        GameStatus::InProgress => game_session.compare_and_swap_status(
            GameStatus::InProgress,
            current_nonce,
            GameStatus::RefundInProgress,
            Some("emergency_refund"),
        )?,
        GameStatus::Expired => game_session.compare_and_swap_status(
            GameStatus::Expired,
            current_nonce,
            GameStatus::RefundInProgress,
            Some("refund_expired"),
        )?,
        _ => {
            msg!(
                "Cannot refund from current status: {:?}",
                game_session.status
            );
            return Err(error!(WagerError::InvalidGameState));
        }
    };

    require!(success, WagerError::ConcurrentOperation);

    msg!(
        "Refund status atomically set to RefundInProgress (nonce: {})",
        game_session.nonce
    );

    let players = game_session.get_all_players();
    let active_players: Vec<Pubkey> = players
        .into_iter()
        .filter(|p| *p != Pubkey::default())
        .collect();

    msg!("Number of active players: {}", active_players.len());
    msg!(
        "Number of remaining accounts: {}",
        ctx.remaining_accounts.len()
    );

    // Validate remaining accounts
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

    // Calculate total refund
    let refund_amount = game_session.session_bet;
    require!(refund_amount > 0, WagerError::InvalidBetAmount);
    require!(
        refund_amount <= 100_000_000_000,
        WagerError::InvalidBetAmount
    );

    let mut total_refund: u64 = 0;
    for _ in 0..active_players.len() {
        total_refund = safe_add_u64(total_refund, refund_amount)?;
    }

    // Validate vault balance
    let vault_balance = ctx.accounts.vault_token_account.amount;
    let min_safety_buffer = 1000u64;
    let percentage_buffer = total_refund / 100; // 1%
    let safety_buffer = std::cmp::max(min_safety_buffer, percentage_buffer);
    require!(
        vault_balance >= total_refund.saturating_add(safety_buffer),
        WagerError::InsufficientVaultFunds
    );

    msg!(
        "Total refund required: {}, Vault balance: {}, Safety buffer: {}",
        total_refund,
        vault_balance,
        safety_buffer
    );

    // Validate remaining accounts
    validate_remaining_accounts_against_players(&ctx.remaining_accounts, &active_players)?;

    // Perform refunds
    let mut total_refunded: u64 = 0;
    let mut successful_refunds: usize = 0;

    for (player_idx, player) in active_players.iter().enumerate() {
        let player_account_idx = player_idx * 2;
        let token_account_idx = player_idx * 2 + 1;

        require!(
            player_account_idx < ctx.remaining_accounts.len()
                && token_account_idx < ctx.remaining_accounts.len(),
            WagerError::InvalidRemainingAccounts
        );

        let player_account = &ctx.remaining_accounts[player_account_idx];
        let player_token_account_info = &ctx.remaining_accounts[token_account_idx];
        let player_token_account = Account::<TokenAccount>::try_from(player_token_account_info)?;

        require!(
            player_token_account.owner == player_account.key(),
            WagerError::InvalidPlayerTokenAccount
        );
        require!(
            player_token_account.mint == TOKEN_ID,
            WagerError::InvalidTokenMint
        );
        require!(player_account.key() == *player, WagerError::InvalidPlayer);

        msg!(
            "Refunding {} to player {} via token account {}",
            refund_amount,
            player,
            player_token_account_info.key()
        );

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
            refund_amount,
        );

        if let Err(e) = transfer_result {
            msg!("Transfer failed for player {}: {:?}", player, e);
            game_session.mark_refund_failed()?;
            return Err(e.into());
        }

        total_refunded = safe_add_u64(total_refunded, refund_amount)?;
        successful_refunds += 1;
    }

    // Final validation
    require!(
        total_refunded == total_refund,
        WagerError::IncompleteDistribution
    );
    require!(
        successful_refunds == active_players.len(),
        WagerError::IncompleteDistribution
    );

    // Mark refund as completed
    game_session.mark_refund_completed()?;

    msg!(
        "Refund completed successfully: {} tokens refunded to {} players",
        total_refunded,
        successful_refunds
    );

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct RefundWager<'info> {
    #[account(
        constraint = game_server.is_signer @ WagerError::UnauthorizedDistribution,
    )]
    pub game_server: Signer<'info>,
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
    /// CHECK: This is a PDA (Program Derived Address) used as the authority for token transfers.
    /// It's validated through the seeds constraint which ensures it's derived from the correct
    /// session_id and game_server. The PDA serves as a secure vault authority and doesn't need
    /// additional type validation since it's only used for signing token transfers, not data access.
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
    )]
    pub vault_token_account: Account<'info, TokenAccount>,
    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}
