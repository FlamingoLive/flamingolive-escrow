use anchor_lang::prelude::*;
use anchor_spl::token::{self, Mint, Token, TokenAccount, Transfer};
use crate::state::*;
use crate::errors::ErrorCode;
use crate::events::*;
use crate::constants::*;

pub fn initialize_config(
    ctx: Context<InitializeConfig>,
    volume_threshold: u64,
    window_duration: i64,
    dispute_window: i64,
    dispute_resolution_deadline: i64,
) -> Result<()> {
    require!(volume_threshold > 0, ErrorCode::InvalidConfig);
    require!(window_duration > 0,  ErrorCode::InvalidConfig);
    require!(dispute_window > 0,   ErrorCode::InvalidConfig);
    require!(dispute_resolution_deadline > 0, ErrorCode::InvalidConfig);

    let config = &mut ctx.accounts.config;
    config.admin               = ctx.accounts.admin.key();
    config.is_paused           = false;
    config.current_volume      = 0;
    config.volume_threshold    = volume_threshold;
    config.last_volume_reset_time = Clock::get()?.unix_timestamp;
    config.window_duration     = window_duration;
    config.dispute_window      = dispute_window;
    config.dispute_resolution_deadline = dispute_resolution_deadline;
    config.platform_fee_vault = ctx.accounts.platform_fee_vault.key();
    config.accumulated_fees    = 0;

    emit!(ConfigInitialized {
        admin:            config.admin,
        volume_threshold,
        window_duration,
        timestamp:        Clock::get()?.unix_timestamp,
    });

    Ok(())
}

pub fn collect_fees(ctx: Context<CollectFees>) -> Result<()> {
    let config = &mut ctx.accounts.config;
    require!(config.accumulated_fees > 0, ErrorCode::InsufficientFunds);

    let fees_to_collect = config.accumulated_fees;
    config.accumulated_fees = 0;

    let bump = ctx.bumps.platform_fee_vault_authority;
    let seeds = &[b"platform_fee_authority".as_ref(), &[bump]];
    let signer = &[&seeds[..]];

    token::transfer(
        CpiContext::new_with_signer(
            ctx.accounts.token_program.to_account_info(),
            Transfer {
                from:      ctx.accounts.platform_fee_vault.to_account_info(),
                to:        ctx.accounts.destination.to_account_info(),
                authority: ctx.accounts.platform_fee_vault_authority.to_account_info(),
            },
            signer,
        ),
        fees_to_collect,
    )?;

    emit!(FeesCollected {
        admin:        ctx.accounts.admin.key(),
        amount:      fees_to_collect,
        timestamp:   Clock::get()?.unix_timestamp,
    });

    Ok(())
}

pub fn update_config(
    ctx: Context<UpdateConfig>,
    is_paused: bool,
    volume_threshold: Option<u64>,
    window_duration: Option<i64>,
    dispute_window: Option<i64>,
    dispute_resolution_deadline: Option<i64>,
) -> Result<()> {
    let config = &mut ctx.accounts.config;
    config.is_paused = is_paused;

    if let Some(vt) = volume_threshold {
        require!(vt > 0, ErrorCode::InvalidConfig);
        config.volume_threshold = vt;
    }
    if let Some(wd) = window_duration {
        require!(wd > 0, ErrorCode::InvalidConfig);
        config.window_duration = wd;
    }
    if let Some(dw) = dispute_window {
        require!(dw > 0, ErrorCode::InvalidConfig);
        config.dispute_window = dw;
    }
    if let Some(drd) = dispute_resolution_deadline {
        require!(drd > 0, ErrorCode::InvalidConfig);
        config.dispute_resolution_deadline = drd;
    }

    emit!(ConfigUpdated {
        is_paused,
        volume_threshold: config.volume_threshold,
        window_duration:  config.window_duration,
        timestamp:        Clock::get()?.unix_timestamp,
    });

    Ok(())
}

#[derive(Accounts)]
pub struct InitializeConfig<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(
        init,
        payer = admin,
        space = 8 + ProgramConfig::INIT_SPACE,
        seeds = [b"config"],
        bump
    )]
    pub config: Box<Account<'info, ProgramConfig>>,

    #[account(
        init,
        payer = admin,
        seeds = [b"platform_fee_vault"],
        bump,
        token::mint = mint,
        token::authority = platform_fee_vault_authority,
    )]
    /// CHECK: Platform fee vault — PDA derived from [b"platform_fee_vault"] seeds
    pub platform_fee_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: PDA authority for platform fee vault
    #[account(
        seeds = [b"platform_fee_authority"],
        bump
    )]
    pub platform_fee_vault_authority: AccountInfo<'info>,

    pub mint: Box<Account<'info, Mint>>,

    pub system_program: Program<'info, System>,
    pub token_program: Program<'info, Token>,
}

#[derive(Accounts)]
pub struct UpdateConfig<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"config"],
        bump,
        has_one = admin
    )]
    pub config: Box<Account<'info, ProgramConfig>>,
}

#[derive(Accounts)]
pub struct CollectFees<'info> {
    pub admin: Signer<'info>,

    #[account(
        mut,
        seeds = [b"config"],
        bump,
        has_one = admin
    )]
    pub config: Box<Account<'info, ProgramConfig>>,

    #[account(
        mut,
        seeds = [b"platform_fee_vault"],
        bump
    )]
    /// CHECK: Platform fee vault — PDA derived from [b"platform_fee_vault"] seeds
    pub platform_fee_vault: Box<Account<'info, TokenAccount>>,

    /// CHECK: Platform fee vault authority
    #[account(
        seeds = [b"platform_fee_authority"],
        bump
    )]
    pub platform_fee_vault_authority: AccountInfo<'info>,

    #[account(
        mut,
        constraint = destination.owner == admin.key()
            @ ErrorCode::InvalidOwner
    )]
    pub destination: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
}