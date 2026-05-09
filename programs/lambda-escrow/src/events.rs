use anchor_lang::prelude::*;
use crate::state::*;

#[event]
pub struct ConfigInitialized {
    pub admin:            Pubkey,
    pub volume_threshold: u64,
    pub window_duration:  i64,
    pub timestamp:        i64,
}

#[event]
pub struct ConfigUpdated {
    pub is_paused:        bool,
    pub volume_threshold: u64,
    pub window_duration:  i64,
    pub timestamp:        i64,
}

#[event]
pub struct EscrowInitialized {
    pub order_code: u64,
    pub buyer:      Pubkey,
    pub seller:     Pubkey,
    pub amount:     u64,
    pub platform_fee: u64,
    pub timestamp:  i64,
}

#[event]
pub struct ShippingConfirmed {
    pub order_code:      u64,
    pub seller:          Pubkey,
    pub tracking_id:     String,
    pub amount_released: u64,
    pub carrier:         Carrier,
    pub timestamp:       i64,
}

#[event]
pub struct DeliveryConfirmed {
    pub order_code:       u64,
    pub buyer:            Pubkey,
    pub seller:           Pubkey,
    pub dispute_deadline: i64,
    pub timestamp:        i64,
}

#[event]
pub struct DisputeRaisedEvent {
    pub order_code: u64,
    pub buyer:      Pubkey,
    pub timestamp:  i64,
}

#[event]
pub struct FundsReleased {
    pub order_code:   u64,
    pub seller:       Pubkey,
    pub amount:       u64,
    /// "auto_release" | "seller_refund" | "adjudge_seller"
    pub release_type: String,
    pub timestamp:    i64,
}

#[event]
pub struct EscrowAdjudicated {
    pub order_code: u64,
    pub judge:      Pubkey,
    /// "buyer" | "seller"
    pub ruled_for:  String,
    pub amount:     u64,
    pub timestamp:  i64,
}

#[event]
pub struct EscrowCancelled {
    pub order_code: u64,
    pub buyer:      Pubkey,
    pub amount:    u64,
    pub timestamp:  i64,
}

#[event]
pub struct PartialRefund {
    pub order_code: u64,
    pub buyer:      Pubkey,
    pub amount:    u64,
    pub timestamp:  i64,
}

#[event]
pub struct CircuitBreakerTriggered {
    pub current_volume: u64,
    pub threshold:      u64,
    pub timestamp:      i64,
}

#[event]
pub struct FeesCollected {
    pub admin:       Pubkey,
    pub amount:      u64,
    pub timestamp:   i64,
}

#[event]
pub struct PartialRefundProcessed {
    pub order_code: u64,
    pub buyer:      Pubkey,
    pub amount:     u64,
    pub timestamp:  i64,
}

#[event]
pub struct DisputeResolved {
    pub order_code: u64,
    pub judge:      Pubkey,
    pub ruled_for:  String,
    pub amount:     u64,
    pub timestamp:  i64,
}
