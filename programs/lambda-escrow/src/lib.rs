use anchor_lang::prelude::*;

pub mod constants;
pub mod errors;
pub mod events;
pub mod instructions;
pub mod state;

use instructions::*;

declare_id!("BcEopLQ9MxMdMtU57m5KYA4sk9qvhy29XkneEKHcfuSf");

#[program]
pub mod lambda_escrow {
    use super::*;

    pub fn initialize_config(
        ctx: Context<InitializeConfig>,
        volume_threshold: u64,
        window_duration: i64,
        dispute_window: i64,
        dispute_resolution_deadline: i64,
    ) -> Result<()> {
        instructions::admin::initialize_config(ctx, volume_threshold, window_duration, dispute_window, dispute_resolution_deadline)
    }

    pub fn update_config(
        ctx: Context<UpdateConfig>,
        is_paused: bool,
        volume_threshold: Option<u64>,
        window_duration: Option<i64>,
        dispute_window: Option<i64>,
        dispute_resolution_deadline: Option<i64>,
    ) -> Result<()> {
        instructions::admin::update_config(ctx, is_paused, volume_threshold, window_duration, dispute_window, dispute_resolution_deadline)
    }

    pub fn collect_fees(ctx: Context<CollectFees>) -> Result<()> {
        instructions::admin::collect_fees(ctx)
    }

    pub fn initialize(
        ctx: Context<Initialize>,
        amount: u64,
        order_code: u64,
        logistics_fee: u64,
    ) -> Result<()> {
        instructions::buyer::initialize(ctx, amount, order_code, logistics_fee)
    }

    pub fn cancel(ctx: Context<Cancel>, order_code: u64) -> Result<()> {
        instructions::buyer::cancel(ctx, order_code)
    }

    pub fn cancel_partial(
        ctx: Context<CancelPartial>,
        order_code: u64,
        amount: u64,
    ) -> Result<()> {
        instructions::buyer::cancel_partial(ctx, order_code, amount)
    }

    pub fn shipping(
        ctx: Context<Shipping>,
        order_code: u64,
        tracking_id: String,
        carrier_code: u8,
    ) -> Result<()> {
        instructions::logistics::shipping(ctx, order_code, tracking_id, carrier_code)
    }

    pub fn delivered(
        ctx: Context<Delivered>,
        order_code: u64,
        tracking_id: String,
    ) -> Result<()> {
        instructions::logistics::delivered(ctx, order_code, tracking_id)
    }

    pub fn raise_dispute(ctx: Context<RaiseDispute>, order_code: u64) -> Result<()> {
        instructions::buyer::raise_dispute(ctx, order_code)
    }

    pub fn exchange(ctx: Context<Exchange>) -> Result<()> {
        instructions::logistics::exchange(ctx)
    }

    pub fn adjudge(ctx: Context<Adjudge>, order_code: u64, status: u8) -> Result<()> {
        instructions::judge::adjudge(ctx, order_code, status)
    }

    pub fn refund(ctx: Context<Refund>, order_code: u64) -> Result<()> {
        instructions::judge::refund(ctx, order_code)
    }

    pub fn refund_partial(
        ctx: Context<RefundPartial>,
        order_code: u64,
        amount: u64,
    ) -> Result<()> {
        instructions::judge::refund_partial(ctx, order_code, amount)
    }
}