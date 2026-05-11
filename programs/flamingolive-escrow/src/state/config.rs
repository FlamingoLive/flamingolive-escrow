use anchor_lang::prelude::*;

#[account]
#[derive(InitSpace)]
pub struct ProgramConfig {
    /// Admin pubkey — only address that can call update_config
    pub admin:                  Pubkey,
    /// Circuit breaker flag — halts all operations when true
    pub is_paused:              bool,
    /// Cumulative volume in current rolling window (USDC base units)
    pub current_volume:         u64,
    /// Volume threshold that triggers circuit breaker
    pub volume_threshold:       u64,
    /// Unix timestamp of last volume window reset
    pub last_volume_reset_time: i64,
    /// Duration of the rolling volume window in seconds
    pub window_duration:        i64,
    /// Platform fee vault authority PDA (seeds: [b"platform_vault"])
    pub platform_fee_vault:     Pubkey,
    /// Accumulated platform fees (USDC base units)
    pub accumulated_fees:       u64,
    /// Dispute window in seconds (buyer must raise dispute within this time after delivery)
    pub dispute_window:         i64,
    /// Dispute resolution deadline in seconds (judge should resolve within this time)
    pub dispute_resolution_deadline: i64,
}
