use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};
use crate::state::*;
use crate::errors::ErrorCode;
use crate::events::*;
use crate::constants::*;

pub fn initialize(
    ctx: Context<Initialize>,
    amount: u64,
    order_code: u64,
    logistics_fee: u64,
) -> Result<()> {
    require!(amount > 0, ErrorCode::InvalidAmount);

    // ── Circuit breaker ───────────────────────────────────────────────
    {
        let config = &mut ctx.accounts.config;
        require!(!config.is_paused, ErrorCode::ProgramPaused);

        let clock = Clock::get()?;
        // Reset rolling window if window_duration has elapsed
        let elapsed = clock.unix_timestamp
            .checked_sub(config.last_volume_reset_time)
            .ok_or(ErrorCode::MathOverflow)?;

        if elapsed >= config.window_duration {
            config.current_volume          = 0;
            config.last_volume_reset_time  = clock.unix_timestamp;
        }

        config.current_volume = config.current_volume
            .checked_add(amount)
            .ok_or(ErrorCode::MathOverflow)?;

        if config.current_volume > config.volume_threshold {
            config.is_paused = true;
            emit!(CircuitBreakerTriggered {
                current_volume: config.current_volume,
                threshold:      config.volume_threshold,
                timestamp:      clock.unix_timestamp,
            });
            return err!(ErrorCode::CircuitBreakerTripped);
        }
    }

    // ── Initialise escrow account ─────────────────────────────────────
    let escrow = &mut ctx.accounts.escrow_account;
    escrow.buyer_key                      = ctx.accounts.buyer.key();
    escrow.buyer_deposit_token_account    = ctx.accounts.buyer_deposit_token_account.key();
    escrow.seller_key                     = ctx.accounts.seller.key();
    escrow.seller_receive_token_account   = ctx.accounts.seller_receive_token_account.key();
    escrow.judge_key                      = ctx.accounts.judge.key();
    escrow.amount                         = amount;
    escrow.order_code                     = order_code;
    escrow.status                         = EscrowStatus::Funded;
    escrow.shipped_time                   = 0;
    escrow.delivery_time                  = 0;
    escrow.dispute_time                   = 0;
    escrow.carrier                        = Carrier::Dhl; // Default
    escrow.tracking_id                    = "".to_string();

    escrow.logistics_fee                  = logistics_fee;

    // Calculate and store platform fee (5%)
    let platform_fee = amount
        .checked_mul(PLATFORM_FEE_PERCENTAGE as u64)
        .ok_or(ErrorCode::MathOverflow)?
        .checked_div(100)
        .ok_or(ErrorCode::MathOverflow)?;
    escrow.platform_fee                   = platform_fee;

    // Remaining deposit goes to vault after logistics is paid
    let escrow_amount = amount
        .checked_sub(logistics_fee)
        .ok_or(ErrorCode::MathOverflow)?;
    escrow.amount                         = escrow_amount;

    // Transfer upfront logistics fee to the platform vault immediately
    let logistics_cpi = Transfer {
        from:      ctx.accounts.buyer_deposit_token_account.to_account_info(),
        to:        ctx.accounts.platform_fee_vault.to_account_info(),
        authority: ctx.accounts.buyer.to_account_info(),
    };
    token::transfer(
        CpiContext::new(ctx.accounts.token_program.to_account_info(), logistics_cpi),
        logistics_fee,
    )?;

    // Note: Platform fee (5%) will be collected at shipping()

    // ── Transfer remaining USDC from buyer to vault ─────────────────────────
    let cpi_accounts = Transfer {
        from:      ctx.accounts.buyer_deposit_token_account.to_account_info(),
        to:        ctx.accounts.vault_account.to_account_info(),
        authority: ctx.accounts.buyer.to_account_info(),
    };
    token::transfer(
        CpiContext::new(ctx.accounts.token_program.to_account_info(), cpi_accounts),
        escrow_amount,
    )?;

    emit!(EscrowInitialized {
        order_code,
        buyer:     ctx.accounts.buyer.key(),
        seller:    ctx.accounts.seller.key(),
        amount: escrow_amount,
        platform_fee,
        timestamp: Clock::get()?.unix_timestamp,
    });

    // ── Token Account Validation ──────────────────────────────────────
    require!(
        !ctx.accounts.buyer_deposit_token_account.is_frozen(),
        ErrorCode::AccountFrozen
    );
    require!(
        !ctx.accounts.seller_receive_token_account.is_frozen(),
        ErrorCode::AccountFrozen
    );
    require!(
        ctx.accounts.buyer_deposit_token_account.mint == ctx.accounts.mint.key(),
        ErrorCode::InvalidMint
    );
    require!(
        ctx.accounts.seller_receive_token_account.mint == ctx.accounts.mint.key(),
        ErrorCode::InvalidMint
    );

    Ok(())
}

pub fn cancel(ctx: Context<Cancel>, order_code: u64) -> Result<()> {
    require!(!ctx.accounts.config.is_paused, ErrorCode::ProgramPaused);

    // Decrement circuit breaker volume
    ctx.accounts.config.current_volume = ctx.accounts.config.current_volume
        .checked_sub(ctx.accounts.escrow_account.amount)
        .ok_or(ErrorCode::MathOverflow)?;

    let order_bytes = order_code.to_le_bytes();
    let judge_key   = ctx.accounts.escrow_account.judge_key;
    let bump        = ctx.bumps.vault_authority;
    let seeds       = &[b"authority", judge_key.as_ref(), order_bytes.as_ref(), &[bump]];
    let signer      = &[&seeds[..]];

    // Return escrow amount to buyer
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
        ctx.accounts.escrow_account.amount,
    )?;

    // Close vault — rent returned to buyer
    token::close_account(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            token::CloseAccount {
                account:     ctx.accounts.vault_account.to_account_info(),
                destination: ctx.accounts.buyer.to_account_info(),
                authority:   ctx.accounts.vault_authority.to_account_info(),
            },
            signer,
        ),
    )?;

    emit!(EscrowCancelled {
        order_code,
        buyer:     ctx.accounts.buyer.key(),
        amount:    ctx.accounts.escrow_account.amount,
        timestamp: Clock::get()?.unix_timestamp,
    });

    ctx.accounts.escrow_account.status = EscrowStatus::Refunded;

    Ok(())
}

pub fn cancel_partial(
    ctx: Context<CancelPartial>,
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
        ctx.accounts.vault_account.amount >= amount,
        ErrorCode::InsufficientFunds
    );
    require!(
        ctx.accounts.escrow_account.status == EscrowStatus::Funded,
        ErrorCode::InvalidStatus
    );

    // Decrement circuit breaker volume
    ctx.accounts.config.current_volume = ctx.accounts.config.current_volume
        .checked_sub(amount)
        .ok_or(ErrorCode::MathOverflow)?;

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

    emit!(PartialRefundProcessed {
        order_code,
        buyer:     ctx.accounts.buyer.key(),
        amount,
        timestamp: Clock::get()?.unix_timestamp,
    });

    Ok(())
}

pub fn raise_dispute(ctx: Context<RaiseDispute>, order_code: u64) -> Result<()> {
    require!(!ctx.accounts.config.is_paused, ErrorCode::ProgramPaused);
    require!(
        ctx.accounts.escrow_account.order_code == order_code,
        ErrorCode::InvalidOrder
    );
    require!(
        ctx.accounts.escrow_account.status == EscrowStatus::Delivered,
        ErrorCode::InvalidStatus
    );

    // Verify buyer is within the configurable dispute window
    let clock = Clock::get()?;
    let deadline = ctx.accounts.escrow_account.delivery_time
        .checked_add(ctx.accounts.config.dispute_window)
        .ok_or(ErrorCode::MathOverflow)?;
    require!(clock.unix_timestamp <= deadline, ErrorCode::DisputeWindowExpired);

    // Record dispute time and update status
    ctx.accounts.escrow_account.status       = EscrowStatus::Disputed;
    ctx.accounts.escrow_account.dispute_time = clock.unix_timestamp;

    emit!(DisputeRaisedEvent {
        order_code,
        buyer:     ctx.accounts.buyer.key(),
        timestamp: clock.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
#[instruction(amount: u64, order_code: u64, logistics_fee: u64)]
pub struct Initialize<'info> {
    #[account(mut, seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    /// CHECK: Seller public key stored in escrow for later validation
    pub seller: AccountInfo<'info>,

    /// CHECK: Judge public key — must be Flamingo oracle backend keypair
    pub judge: AccountInfo<'info>,

    pub mint: Box<Account<'info, Mint>>,

    #[account(
        init,
        seeds = [b"vault", judge.key().as_ref(), order_code.to_le_bytes().as_ref()],
        bump,
        payer = buyer,
        token::mint = mint,
        token::authority = vault_authority,
    )]
    pub vault_account: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA authority — derived deterministically, validated by seeds
    #[account(
        seeds = [b"authority", judge.key().as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub vault_authority: AccountInfo<'info>,

    #[account(
        mut,
        constraint = buyer_deposit_token_account.amount >= amount
            @ ErrorCode::InsufficientFunds,
        constraint = buyer_deposit_token_account.owner == buyer.key()
            @ ErrorCode::InvalidOwner,
    )]
    pub buyer_deposit_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        constraint = seller_receive_token_account.owner == seller.key()
            @ ErrorCode::InvalidOwner
    )]
    pub seller_receive_token_account: Box<Account<'info, TokenAccount>>,

    #[account(
        init,
        payer = buyer,
        space = 8 + EscrowAccount::INIT_SPACE,
        seeds = [b"escrow", judge.key().as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,

    /// CHECK: Platform fee vault — PDA derived from [b"platform_vault"]
    #[account(
        mut,
        constraint = platform_fee_vault.key() == config.platform_fee_vault
            @ ErrorCode::InvalidAccount
    )]
    pub platform_fee_vault: Box<Account<'info, TokenAccount>>,

    pub system_program: Program<'info, System>,
    pub token_program:  Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(order_code: u64)]
pub struct Cancel<'info> {
    #[account(mut, seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    /// Flamingo logistics oracle must sign to authorize cancellation
    #[account(
        mut,
        constraint = escrow_account.judge_key == judge.key()
            @ ErrorCode::InvalidOwner
    )]
    pub judge: Signer<'info>,

    /// CHECK: Buyer receives the refund — not required to sign
    #[account(mut)]
    pub buyer: AccountInfo<'info>,

    #[account(
        mut,
        constraint = buyer_deposit_token_account.owner == buyer.key()
            @ ErrorCode::InvalidOwner
    )]
    pub buyer_deposit_token_account: Box<Account<'info, TokenAccount>>,

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
        constraint = escrow_account.buyer_key == buyer.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.buyer_deposit_token_account == buyer_deposit_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.order_code == order_code
            @ ErrorCode::InvalidOrder,
        constraint = escrow_account.status == EscrowStatus::Funded
            @ ErrorCode::InvalidStatus,
        seeds = [b"escrow", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump,
        close = buyer
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(order_code: u64, amount: u64)]
pub struct CancelPartial<'info> {
    #[account(mut, seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    /// Flamingo logistics oracle must sign to authorize cancellation
    #[account(
        mut,
        constraint = escrow_account.judge_key == judge.key()
            @ ErrorCode::InvalidOwner
    )]
    pub judge: Signer<'info>,

    /// CHECK: Buyer receives the partial refund
    pub buyer: AccountInfo<'info>,

    #[account(
        mut,
        constraint = buyer_deposit_token_account.owner == buyer.key()
            @ ErrorCode::InvalidOwner
    )]
    pub buyer_deposit_token_account: Box<Account<'info, TokenAccount>>,

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
        constraint = escrow_account.buyer_key == buyer.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.buyer_deposit_token_account == buyer_deposit_token_account.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.order_code == order_code
            @ ErrorCode::InvalidOrder,
        constraint = escrow_account.status == EscrowStatus::Funded
            @ ErrorCode::InvalidStatus,
        constraint = escrow_account.amount >= amount
            @ ErrorCode::InsufficientFunds,
        seeds = [b"escrow", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,

    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
#[instruction(order_code: u64)]
pub struct RaiseDispute<'info> {
    #[account(seeds = [b"config"], bump)]
    pub config: Box<Account<'info, ProgramConfig>>,

    #[account(mut)]
    pub buyer: Signer<'info>,

    #[account(
        mut,
        constraint = escrow_account.buyer_key == buyer.key()
            @ ErrorCode::InvalidOwner,
        constraint = escrow_account.order_code == order_code
            @ ErrorCode::InvalidOrder,
        constraint = escrow_account.status == EscrowStatus::Delivered
            @ ErrorCode::InvalidStatus,
        seeds = [b"escrow", escrow_account.judge_key.as_ref(), order_code.to_le_bytes().as_ref()],
        bump
    )]
    pub escrow_account: Box<Account<'info, EscrowAccount>>,
}
