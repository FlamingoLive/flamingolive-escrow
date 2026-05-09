use anchor_lang::prelude::*;
use anchor_spl::token::{self, Token, TokenAccount, Transfer, CloseAccount};
use crate::state::*;
use crate::errors::ErrorCode;
use crate::events::*;
use crate::constants::*;

pub fn shipping(
    ctx: Context<Shipping>,
    order_code: u64,
    tracking_id: String,
    carrier_code: u8,
) -> Result<()> {
    require!(!ctx.accounts.config.is_paused, ErrorCode::ProgramPaused);
    require!(
        ctx.accounts.escrow_account.order_code == order_code,
        ErrorCode::InvalidOrder
    );
    require!(
        ctx.accounts.escrow_account.status == EscrowStatus::Funded,
        ErrorCode::InvalidStatus
    );

    // Validate tracking ID
    require!(
        tracking_id.len() >= TRACKING_ID_MIN_LEN as usize,
        ErrorCode::InvalidTrackingId
    );
    require!(
        tracking_id.len() <= TRACKING_ID_MAX_LEN as usize,
        ErrorCode::InvalidTrackingId
    );

    // Validate carrier code
    require!(carrier_code <= MAX_CARRIER_CODE, ErrorCode::InvalidCarrierCode);
    require!(
        ctx.accounts.vault_account.amount >= ctx.accounts.escrow_account.amount,
        ErrorCode::InsufficientFunds
    );

    // Extract platform fee from escrow
    let platform_fee = ctx.accounts.escrow_account.platform_fee;

    // Remaining for delivery: remaining escrow amount minus platform fee
    let remaining_after_fees = ctx.accounts.escrow_account.amount
        .checked_sub(platform_fee)
        .ok_or(ErrorCode::MathOverflow)?;

    // Calculate seller share: 50% of remaining
    let seller_share = remaining_after_fees
        .checked_div(2)
        .ok_or(ErrorCode::MathOverflow)?;

    // Remaining for delivery: remaining 50%
    let remaining = remaining_after_fees
        .checked_sub(seller_share)
        .ok_or(ErrorCode::MathOverflow)?;

    // Update accumulated fees
    ctx.accounts.config.accumulated_fees = ctx.accounts.config.accumulated_fees
        .checked_add(platform_fee)
        .ok_or(ErrorCode::MathOverflow)?;

    // Update escrow state
    let clock = Clock::get()?;
    ctx.accounts.escrow_account.status       = EscrowStatus::Shipped;
    ctx.accounts.escrow_account.tracking_id  = tracking_id.clone();
    
    // Map carrier code to enum
    ctx.accounts.escrow_account.carrier = match carrier_code {
        0 => Carrier::Dhl,
        1 => Carrier::Aramex,
        2 => Carrier::Fedex,
        3 => Carrier::Sendbox,
        _ => return err!(ErrorCode::InvalidCarrierCode),
    };

    ctx.accounts.escrow_account.shipped_time = clock.unix_timestamp;
    ctx.accounts.escrow_account.amount       = remaining;

    // Release seller net to seller
    let order_bytes = order_code.to_le_bytes();
    let judge_key   = ctx.accounts.escrow_account.judge_key;
    let bump        = ctx.bumps.vault_authority;
    let seeds       = &[b"authority", judge_key.as_ref(), order_bytes.as_ref(), &[bump]];
    let signer      = &[&seeds[..]];

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
        seller_share,
    )?;

    // Transfer platform fee to platform vault
    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from:      ctx.accounts.vault_account.to_account_info(),
                to:        ctx.accounts.platform_fee_vault.to_account_info(),
                authority: ctx.accounts.vault_authority.to_account_info(),
            },
            signer,
        ),
        platform_fee,
    )?;

    emit!(ShippingConfirmed {
        order_code,
        seller:         ctx.accounts.seller.key(),
        tracking_id,
        amount_released: seller_share,
        carrier:        ctx.accounts.escrow_account.carrier,
        timestamp:      clock.unix_timestamp,
    });

    Ok(())
}

pub fn delivered(
    ctx: Context<Delivered>,
    order_code: u64,
    tracking_id: String,
) -> Result<()> {
    require!(!ctx.accounts.config.is_paused, ErrorCode::ProgramPaused);
    require!(
        ctx.accounts.escrow_account.order_code == order_code,
        ErrorCode::InvalidOrder
    );
    require!(
        ctx.accounts.escrow_account.status == EscrowStatus::Shipped,
        ErrorCode::InvalidStatus
    );

    // Cross-check: tracking_id must match what was registered at shipping
    require!(
        ctx.accounts.escrow_account.tracking_id == tracking_id,
        ErrorCode::TrackingIdMismatch
    );

    let clock = Clock::get()?;
    ctx.accounts.escrow_account.status        = EscrowStatus::Delivered;
    ctx.accounts.escrow_account.delivery_time = clock.unix_timestamp;

    let dispute_deadline = clock.unix_timestamp
        .checked_add(ctx.accounts.config.dispute_window)
        .ok_or(ErrorCode::MathOverflow)?;

    emit!(DeliveryConfirmed {
        order_code,
        buyer:            ctx.accounts.buyer.key(),
        seller:           ctx.accounts.seller.key(),
        dispute_deadline,
        timestamp:        clock.unix_timestamp,
    });

    Ok(())
}

pub fn exchange(ctx: Context<Exchange>) -> Result<()> {
    require!(!ctx.accounts.config.is_paused, ErrorCode::ProgramPaused);

    // Verify configurable dispute window has fully elapsed
    let clock = Clock::get()?;
    let unlock_time = ctx.accounts.escrow_account.delivery_time
        .checked_add(ctx.accounts.config.dispute_window)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(clock.unix_timestamp >= unlock_time, ErrorCode::InDisputeWindow);

    // Only status DELIVERED (no dispute) — DISPUTED is explicitly blocked
    require!(ctx.accounts.escrow_account.status == EscrowStatus::Delivered, ErrorCode::InvalidStatus);
    require!(
        ctx.accounts.vault_account.amount >= ctx.accounts.escrow_account.amount,
        ErrorCode::InsufficientFunds
    );

    let order_bytes = ctx.accounts.escrow_account.order_code.to_le_bytes();
    let judge_key   = ctx.accounts.escrow_account.judge_key;
    let bump        = ctx.bumps.vault_authority;
    let seeds       = &[b"authority", judge_key.as_ref(), order_bytes.as_ref(), &[bump]];
    let signer      = &[&seeds[..]];

    let amount = ctx.accounts.escrow_account.amount;

    // Release remaining 50% to seller
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

    ctx.accounts.escrow_account.status = EscrowStatus::Released;

    // Close vault — rent returned to buyer
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

    emit!(FundsReleased {
        order_code:   ctx.accounts.escrow_account.order_code,
        seller:       ctx.accounts.seller.key(),
        amount,
        release_type: "auto_release".to_string(),
        timestamp:    clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(order_code: u64, tracking_id: String, carrier_code: u8)]
pub struct Shipping<'info> {
    #[account(mut, seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    /// CHECK: Buyer is a passive party — not required to sign
    pub buyer: AccountInfo<'info>,

    #[account(
        constraint = buyer_deposit_token_account.owner == buyer.key()
            @ ErrorCode::InvalidOwner
    )]
    pub buyer_deposit_token_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: Seller is the recipient of the 45% payment
    pub seller: AccountInfo<'info>,

    #[account(
        mut,
        constraint = seller_receive_token_account.owner == seller.key()
            @ ErrorCode::InvalidOwner
    )]
    pub seller_receive_token_account: Box<Account<'info, TokenAccount>>,

    /// Flamingo logistics oracle backend — must match judge_key stored in escrow
    #[account(
        mut,
        constraint = escrow_account.judge_key == judge.key()
            @ ErrorCode::InvalidOwner
    )]
    pub judge: Signer<'info>,

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
        constraint = escrow_account.order_code == order_code
            @ ErrorCode::InvalidOrder,
        constraint = escrow_account.status == EscrowStatus::Funded
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

    /// CHECK: Platform fee vault — PDA derived from [b"platform_vault"]
    #[account(
        mut,
        constraint = platform_fee_vault.key() == config.platform_fee_vault
            @ ErrorCode::InvalidAccount
    )]
    pub platform_fee_vault: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(order_code: u64, tracking_id: String)]
pub struct Delivered<'info> {
    #[account(seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    /// Flamingo logistics oracle — must match judge_key stored in escrow
    #[account(
        mut,
        constraint = escrow_account.judge_key == judge.key()
            @ ErrorCode::InvalidOwner
    )]
    pub judge: Signer<'info>,

    /// CHECK: Buyer — passive party
    pub buyer: AccountInfo<'info>,

    /// CHECK: Seller — passive party
    pub seller: AccountInfo<'info>,

    #[account(
        mut,
        constraint = escrow_account.buyer_key == buyer.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_key == seller.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.order_code == order_code
            @ ErrorCode::InvalidOrder,
        constraint = escrow_account.status == EscrowStatus::Shipped
            @ ErrorCode::InvalidStatus,
        seeds = [b"escrow", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,
}

#[derive(Accounts)]
pub struct Exchange<'info> {
    #[account(seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    /// Flamingo backend executes auto-release after dispute window expires
    #[account(
        mut,
        constraint = escrow_account.judge_key == judge.key()
            @ ErrorCode::InvalidOwner
    )]
    pub judge: Signer<'info>,

    /// CHECK: Rent returned to buyer on vault closure
    #[account(mut)]
    pub buyer: AccountInfo<'info>,

    /// CHECK: Seller — passive party
    pub seller: AccountInfo<'info>,

    #[account(
        mut,
        constraint = seller_receive_token_account.owner == seller.key()
            @ ErrorCode::InvalidOwner
    )]
    pub seller_receive_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        mut,
        constraint = escrow_account.buyer_key == buyer.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_receive_token_account == seller_receive_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.seller_key == seller.key()
            @ ErrorCode::InvalidOwner,
        // Only status Delivered — disputed orders are explicitly blocked
        constraint = escrow_account.status == EscrowStatus::Delivered
            @ ErrorCode::InvalidStatus,
        seeds = [b"escrow", escrow_account.judge_key.as_ref(), escrow_account.order_code.to_le_bytes().as_ref()],
        bump,
        close = buyer
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,

    #[account(
        mut,
        seeds = [b"vault", escrow_account.judge_key.as_ref(), escrow_account.order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA authority
    #[account(
        seeds = [b"authority", escrow_account.judge_key.as_ref(), escrow_account.order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_authority: AccountInfo<'info>,

    #[account(
        mut,
        constraint = platform_fee_vault.key() == config.platform_fee_vault
            @ ErrorCode::InvalidAccount
    )]
    pub platform_fee_vault: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}
