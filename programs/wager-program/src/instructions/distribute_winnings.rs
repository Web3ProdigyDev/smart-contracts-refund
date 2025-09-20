// distribute_winnings.rs - SECURITY HARDENED VERSION WITH ARITHMETIC FIXES
use crate::{errors::WagerError, state::*, TOKEN_ID, utils::{validate_session_id, validate_remaining_accounts_against_players}};
use anchor_lang::prelude::*;
use anchor_spl::associated_token::AssociatedToken;
use anchor_spl::token::{Token, TokenAccount};

pub fn distribute_pay_spawn_earnings<'info>(
    ctx: Context<'_, '_, 'info, 'info, DistributeWinnings<'info>>,
    session_id: String,
) -> Result<()> {
    let game_session = &mut ctx.accounts.game_session;
    let vault_bump = game_session.vault_bump; // Store vault_bump early
    
    // Validate session_id format FIRST
    validate_session_id(&session_id)?;
    
    // CRITICAL FIX: Prevent double distribution by checking status first
    require!(
        game_session.status == GameStatus::InProgress,
        WagerError::InvalidGameState
    );
    
    // Mark as completed IMMEDIATELY to prevent reentrancy
    game_session.status = GameStatus::Completed;
    
    msg!("Starting distribution for session: {}", session_id);

    let players = game_session.get_all_players();
    msg!("Number of players: {}", players.len());
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

    // CRITICAL FIX: Calculate total required funds FIRST and verify vault balance
    let mut total_required: u64 = 0;
    let mut player_earnings: Vec<(Pubkey, u64)> = Vec::new();
    
    for player in players {
        if player == Pubkey::default() {
            continue;
        }
        
        let kills_and_spawns = game_session.get_kills_and_spawns(player)?;
        if kills_and_spawns == 0 {
            continue;
        }

        // CRITICAL FIX: Use checked arithmetic with bounds validation to prevent overflow
        let kills_and_spawns_u64 = kills_and_spawns as u64;
        
        // Check if multiplication would overflow
        require!(
            kills_and_spawns_u64 <= u64::MAX / game_session.session_bet,
            WagerError::ArithmeticError
        );
        
        let product = kills_and_spawns_u64
            .checked_mul(game_session.session_bet)
            .ok_or(WagerError::ArithmeticError)?;
            
        // Check if division is safe (product should be at least 10 for meaningful result)
        require!(product >= 10, WagerError::ArithmeticError);
        
        let earnings = product
            .checked_div(10)
            .ok_or(WagerError::ArithmeticError)?;
            
        // Check if adding to total would overflow
        require!(
            total_required <= u64::MAX - earnings,
            WagerError::ArithmeticError
        );
            
        total_required = total_required
            .checked_add(earnings)
            .ok_or(WagerError::ArithmeticError)?;
            
        player_earnings.push((player, earnings));
    }
    
    // CRITICAL FIX: Verify vault has sufficient balance BEFORE any transfers
    require!(
        ctx.accounts.vault_token_account.amount >= total_required,
        WagerError::InsufficientVaultFunds
    );
    
    msg!("Total required: {}, Vault balance: {}", total_required, ctx.accounts.vault_token_account.amount);

    // CRITICAL FIX: Validate ALL remaining accounts against actual players
    validate_remaining_accounts_against_players(
        &ctx.remaining_accounts,
        &player_earnings.iter().map(|(pubkey, _)| *pubkey).collect::<Vec<_>>()
    )?;

    // Now perform distributions with validated accounts
    for (player, earnings) in player_earnings {
        if earnings == 0 {
            continue;
        }

        // CRITICAL FIX: Find player account with strict validation
        let player_index = ctx
            .remaining_accounts
            .iter()
            .step_by(2)
            .position(|acc| acc.key() == player)
            .ok_or(WagerError::InvalidPlayer)?;

        let player_account = &ctx.remaining_accounts[player_index * 2];
        let player_token_account_info = &ctx.remaining_accounts[player_index * 2 + 1];
        let player_token_account = Account::<TokenAccount>::try_from(player_token_account_info)?;

        // CRITICAL FIX: Strict token account validation
        require!(
            player_token_account.owner == player_account.key(),
            WagerError::InvalidPlayerTokenAccount
        );

        require!(
            player_token_account.mint == TOKEN_ID,
            WagerError::InvalidTokenMint
        );

        // CRITICAL FIX: Verify the account is actually who they claim to be
        require!(
            player_account.key() == player,
            WagerError::InvalidPlayer
        );

        msg!("Transferring {} to player {}", earnings, player);

        // Perform the transfer
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
                    &[vault_bump],
                ]],
            ),
            earnings,
        )?;
    }

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
    
    msg!("Starting distribution for session: {}", session_id);

    // Verify authority
    require!(
        game_session.authority == ctx.accounts.game_server.key(),
        WagerError::UnauthorizedDistribution
    );
    
    // CRITICAL FIX: Check game status and prevent double distribution
    require!(
        game_session.status == GameStatus::InProgress,
        WagerError::InvalidGameState
    );
    
    // Mark as completed FIRST to prevent reentrancy
    game_session.status = GameStatus::Completed;

    // Validate winning team selection
    require!(
        winning_team == 0 || winning_team == 1,
        WagerError::InvalidWinningTeam
    );

    let players_per_team = game_session.game_mode.players_per_team();
    let vault_bump = game_session.vault_bump; // Store vault_bump early

    // CRITICAL FIX: Validate both teams are actually full before distribution
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

    // Get winner account and token account from remaining accounts
    require!(
        ctx.remaining_accounts.len() >= 2 * players_per_team,
        WagerError::InvalidRemainingAccounts
    );

    // CRITICAL FIX: Calculate total payout with bounds checking to prevent overflow
    let session_bet = game_session.session_bet;
    let players_per_team_u64 = players_per_team as u64;
    
    // Check if session_bet * 2 would overflow
    require!(
        session_bet <= u64::MAX / 2,
        WagerError::ArithmeticError
    );
    
    let winning_amount = session_bet
        .checked_mul(2)
        .ok_or(WagerError::ArithmeticError)?;
    
    // Check if winning_amount * players_per_team would overflow
    require!(
        winning_amount <= u64::MAX / players_per_team_u64,
        WagerError::ArithmeticError
    );
        
    let total_payout = winning_amount
        .checked_mul(players_per_team_u64)
        .ok_or(WagerError::ArithmeticError)?;
        
    require!(
        ctx.accounts.vault_token_account.amount >= total_payout,
        WagerError::InsufficientVaultFunds
    );

    // CRITICAL FIX: Validate remaining accounts match winning players exactly
    for i in 0..players_per_team {
        let expected_winner = winning_players[i];
        let provided_account = &ctx.remaining_accounts[i * 2];
        
        require!(
            provided_account.key() == expected_winner,
            WagerError::InvalidWinner
        );
    }

    for i in 0..players_per_team {
        let winner = &ctx.remaining_accounts[i * 2];
        let winner_token_account_info = &ctx.remaining_accounts[i * 2 + 1];
        let winner_token_account = Account::<TokenAccount>::try_from(winner_token_account_info)?;

        // Verify winner constraints
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
            winning_players
                .iter()
                .any(|&p| p == winner_pubkey),
            WagerError::InvalidWinner
        );

        msg!("Transferring {} to winner {}", winning_amount, winner_pubkey);

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
                    &[vault_bump],
                ]],
            ),
            winning_amount,
        )?;
    }

    Ok(())
}

#[derive(Accounts)]
#[instruction(session_id: String)]
pub struct DistributeWinnings<'info> {
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