use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, CloseAccount};
use crate::state::*;
use crate::errors::ErrorCode;
use crate::events::*;

pub fn adjudge(ctx: Context<Adjudge>, order_code: u64, status: u8) -> Result<()> {
    require!(!ctx.accounts.config.is_paused, ErrorCode::ProgramPaused);
    require!(
        ctx.accounts.escrow_account.order_code == order_code,
        ErrorCode::InvalidOrder
    );
    require!(
        ctx.accounts.escrow_account.status == EscrowStatus::Disputed,
        ErrorCode::InvalidStatus
    );
    
    // Verify judge is acting within the resolution deadline
    let clock = Clock::get()?;
    let deadline = ctx.accounts.escrow_account.dispute_time
        .checked_add(ctx.accounts.config.dispute_resolution_deadline)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(clock.unix_timestamp <= deadline, ErrorCode::DisputeWindowExpired); // Or a new error code
    require!(status == 0 || status == 2, ErrorCode::InvalidStatus);
    require!(
        ctx.accounts.vault_account.amount >= ctx.accounts.escrow_account.amount,
        ErrorCode::InsufficientFunds
    );

    let order_bytes = order_code.to_le_bytes();
    let judge_key   = ctx.accounts.judge.key();
    let bump        = ctx.bumps.vault_authority;
    let seeds       = &[b"authority", judge_key.as_ref(), order_bytes.as_ref(), &[bump]];
    let signer      = &[&seeds[..]];

    let amount      = ctx.accounts.escrow_account.amount;

    if status == 0 {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from:      ctx.accounts.vault_account.to_account_info(),
                    to:        ctx.accounts.buyer_deposit_token_account.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer,
            ),
            amount,
        )?;
    } else {
        token::transfer(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                Transfer {
                    from:      ctx.accounts.vault_account.to_account_info(),
                    to:        ctx.accounts.seller_receive_token_account.to_account_info(),
                    authority: ctx.accounts.vault_authority.to_account_info(),
                },
                signer,
            ),
            amount,
        )?;
    }

    ctx.accounts.escrow_account.status = EscrowStatus::Adjudicated;

    let ruled_for = if status == 0 {
        "buyer".to_string()
    } else {
        "seller".to_string()
    };

    emit!(DisputeResolved {
        order_code,
        judge:     ctx.accounts.judge.key(),
        ruled_for,
        amount,
        timestamp: Clock::get()?.unix_timestamp,
    });

    token::close_account(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account:     ctx.accounts.vault_account.to_account_info(),
                destination: ctx.accounts.buyer.to_account_info(),
                authority:   ctx.accounts.vault_authority.to_account_info(),
            },
            signer,
        ),
    )?;

    Ok(())
}

pub fn refund(ctx: Context<Refund>, order_code: u64) -> Result<()> {
    require!(!ctx.accounts.config.is_paused, ErrorCode::ProgramPaused);
    require!(
        ctx.accounts.escrow_account.order_code == order_code,
        ErrorCode::InvalidOrder
    );
    require!(
        ctx.accounts.escrow_account.status == EscrowStatus::Shipped
            || ctx.accounts.escrow_account.status == EscrowStatus::Delivered
            || ctx.accounts.escrow_account.status == EscrowStatus::Disputed,
        ErrorCode::InvalidStatus
    );
    require!(
        ctx.accounts.vault_account.amount >= ctx.accounts.escrow_account.amount,
        ErrorCode::InsufficientFunds
    );

    // Use deposited_amount for consistent circuit-breaker decrement —
    // escrow.amount may already reflect partial 50% release at shipping.
    ctx.accounts.config.current_volume = ctx.accounts.config.current_volume
        .checked_sub(ctx.accounts.escrow_account.deposited_amount)
        .ok_or(ErrorCode::MathOverflow)?;

    let order_bytes = order_code.to_le_bytes();
    let judge_key   = ctx.accounts.judge.key();
    let bump        = ctx.bumps.vault_authority;
    let seeds       = &[b"authority", judge_key.as_ref(), order_bytes.as_ref(), &[bump]];
    let signer      = &[&seeds[..]];

    let vault_amount = ctx.accounts.escrow_account.amount;

    // Return vault balance to buyer
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from:      ctx.accounts.vault_account.to_account_info(),
                to:        ctx.accounts.buyer_deposit_token_account.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signer,
        ),
        vault_amount,
    )?;

    // The platform fee was collected from the vault at shipping(); return it.
    let platform_fee = ctx.accounts.escrow_account.platform_fee;
    let bump_platform = ctx.bumps.platform_fee_vault_authority;
    let seeds_platform: &[&[u8]] = &[b"platform_fee_authority", &[bump_platform]];
    let signer_platform = &[&seeds_platform[..]];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from:      ctx.accounts.platform_fee_vault.to_account_info(),
                to:        ctx.accounts.buyer_deposit_token_account.to_account_info(),
                authority: ctx.accounts.platform_fee_vault_authority.to_account_info(),
            },
            signer_platform,
        ),
        platform_fee,
    )?;

    ctx.accounts.config.accumulated_fees = ctx.accounts.config.accumulated_fees
        .checked_sub(platform_fee)
        .ok_or(ErrorCode::MathOverflow)?;

    token::close_account(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            CloseAccount {
                account:     ctx.accounts.vault_account.to_account_info(),
                destination: ctx.accounts.buyer.to_account_info(),
                authority:   ctx.accounts.vault_authority.to_account_info(),
            },
            signer,
        ),
    )?;

    emit!(EscrowRefunded {
        order_code,
        buyer:     ctx.accounts.buyer.key(),
        amount:    vault_amount
            .checked_add(platform_fee)
            .ok_or(ErrorCode::MathOverflow)?,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

pub fn refund_partial(
    ctx: Context<RefundPartial>,
    order_code: u64,
    amount: u64,
) -> Result<()> {
    require!(!ctx.accounts.config.is_paused, ErrorCode::ProgramPaused);
    require!(amount > 0, ErrorCode::InvalidAmount);
    require!(
        ctx.accounts.escrow_account.amount >= amount,
        ErrorCode::InsufficientFunds
    );
    require!(
        ctx.accounts.escrow_account.status == EscrowStatus::Shipped 
            || ctx.accounts.escrow_account.status == EscrowStatus::Delivered
            || ctx.accounts.escrow_account.status == EscrowStatus::Disputed,
        ErrorCode::InvalidStatus
    );
    require!(
        ctx.accounts.vault_account.amount >= amount,
        ErrorCode::InsufficientFunds
    );

    ctx.accounts.config.current_volume = ctx.accounts.config.current_volume
        .checked_sub(amount)
        .ok_or(ErrorCode::MathOverflow)?;

    let order_bytes = order_code.to_le_bytes();
    let judge_key   = ctx.accounts.judge.key();
    let bump        = ctx.bumps.vault_authority;
    let seeds       = &[b"authority", judge_key.as_ref(), order_bytes.as_ref(), &[bump]];
    let signer      = &[&seeds[..]];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from:      ctx.accounts.vault_account.to_account_info(),
                to:        ctx.accounts.buyer_deposit_token_account.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signer,
        ),
        amount,
    )?;

    ctx.accounts.escrow_account.amount = ctx.accounts.escrow_account.amount
        .checked_sub(amount)
        .ok_or(ErrorCode::MathOverflow)?;

    // Keep deposited_amount in sync so a subsequent full refund decrements correctly.
    ctx.accounts.escrow_account.deposited_amount = ctx.accounts.escrow_account.deposited_amount
        .saturating_sub(amount);

    let remaining = ctx.accounts.escrow_account.amount;

    if remaining == 0 {
        ctx.accounts.escrow_account.status = EscrowStatus::Refunded;

        token::close_account(
            CpiContext::new_with_signer(
                ctx.accounts.token_program.to_account_info(),
                CloseAccount {
                    account:     ctx.accounts.vault_account.to_account_info(),
                    destination: ctx.accounts.buyer.to_account_info(),
                    authority:   ctx.accounts.vault_authority.to_account_info(),
                },
                signer,
            ),
        )?;
    }

    emit!(PartialRefundProcessed {
        order_code:   ctx.accounts.escrow_account.order_code,
        buyer:      ctx.accounts.buyer.key(),
        amount,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(order_code: u64, status: u8)]
pub struct Adjudge<'info> {
    #[account(seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    #[account(mut)]
    pub judge: Signer<'info>,

    /// CHECK: Buyer — may receive refund
    #[account(mut)]
    pub buyer: AccountInfo<'info>,

    #[account(
        mut,
        constraint = buyer_deposit_token_account.owner == buyer.key()
            @ ErrorCode::InvalidOwner
    )]
    pub buyer_deposit_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Seller — may receive release
    #[account(mut)]
    pub seller: AccountInfo<'info>,

    #[account(
        mut,
        constraint = seller_receive_token_account.owner == seller.key()
            @ ErrorCode::InvalidOwner
    )]
    pub seller_receive_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = escrow_account.buyer_deposit_token_account == buyer_deposit_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.buyer_key == buyer.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_receive_token_account == seller_receive_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_key == seller.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.judge_key == judge.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.order_code == order_code
            @ ErrorCode::InvalidOrder,
        seeds = [b"escrow", judge.key().as_ref(), order_code.to_le_bytes().as_ref()],
        bump,
        close = buyer
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,

    #[account(
        mut,
        seeds = [b"vault", judge.key().as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA authority — uses "authority" seeds
    #[account(
        seeds = [b"authority", judge.key().as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(order_code: u64)]
pub struct Refund<'info> {
    #[account(mut, seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    #[account(mut)]
    pub judge: Signer<'info>,

    /// CHECK: buyer may receive refund
    #[account(mut)]
    pub buyer: AccountInfo<'info>,

    #[account(
        mut,
        constraint = buyer_deposit_token_account.owner == buyer.key()
            @ ErrorCode::InvalidOwner
    )]
    pub buyer_deposit_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: seller may receive release
    pub seller: AccountInfo<'info>,

    #[account(
        constraint = seller_receive_token_account.owner == seller.key()
            @ ErrorCode::InvalidOwner
    )]
    pub seller_receive_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = escrow_account.buyer_deposit_token_account == buyer_deposit_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.buyer_key == buyer.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_receive_token_account == seller_receive_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_key == seller.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.judge_key == judge.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.order_code == order_code
            @ ErrorCode::InvalidOrder,
        constraint = escrow_account.status == EscrowStatus::Shipped || escrow_account.status == EscrowStatus::Delivered || escrow_account.status == EscrowStatus::Disputed
            @ ErrorCode::InvalidStatus,
        seeds = [b"escrow", judge.key().as_ref(), order_code.to_le_bytes().as_ref()],
        bump,
        close = buyer
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,

    #[account(
        mut,
        seeds = [b"vault", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA authority
    #[account(
        seeds = [b"authority", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_authority: AccountInfo<'info>,

    #[account(
        mut,
        constraint = platform_fee_vault.key() == config.platform_fee_vault
            @ ErrorCode::InvalidAccount
    )]
    pub platform_fee_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA authority for the platform fee vault
    #[account(
        seeds = [b"platform_fee_authority"],
        bump
    )]
    pub platform_fee_vault_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(order_code: u64, amount: u64)]
pub struct RefundPartial<'info> {
    #[account(mut, seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    #[account(mut)]
    pub judge: Signer<'info>,

    /// CHECK: buyer may receive partial refund
    pub buyer: AccountInfo<'info>,

    #[account(
        mut,
        constraint = buyer_deposit_token_account.owner == buyer.key()
            @ ErrorCode::InvalidOwner
    )]
    pub buyer_deposit_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: seller may receive partial release
    pub seller: AccountInfo<'info>,

    #[account(
        constraint = seller_receive_token_account.owner == seller.key()
            @ ErrorCode::InvalidOwner
    )]
    pub seller_receive_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = escrow_account.buyer_deposit_token_account == buyer_deposit_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.buyer_key == buyer.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_receive_token_account == seller_receive_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_key == seller.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.judge_key == judge.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.order_code == order_code
            @ ErrorCode::InvalidOrder,
        constraint = escrow_account.amount >= amount
            @ ErrorCode::InsufficientFunds,
        constraint = escrow_account.status == EscrowStatus::Shipped || escrow_account.status == EscrowStatus::Delivered || escrow_account.status == EscrowStatus::Disputed
            @ ErrorCode::InvalidStatus,
        seeds = [b"escrow", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,

    #[account(
        mut,
        seeds = [b"vault", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA authority
    #[account(
        seeds = [b"authority", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_authority: AccountInfo<'info>,

    pub token_program: Program<'info, Token>,
}
