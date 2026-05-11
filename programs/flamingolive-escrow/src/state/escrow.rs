use anchor_lang::prelude::*;

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum EscrowStatus {
    Funded,
    Shipped,
    Delivered,
    Disputed,
    Released,
    Adjudicated,
    Refunded,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, PartialEq, Eq, InitSpace)]
pub enum Carrier {
    Dhl,
    Aramex,
    Fedex,
    Sendbox,
}

#[account]
#[derive(InitSpace)]
pub struct EscrowAccount {
    /// Buyer's wallet pubkey
    pub buyer_key:                    Pubkey,
    /// Buyer's USDC token account
    pub buyer_deposit_token_account:  Pubkey,
    /// Seller's wallet pubkey
    pub seller_key:                   Pubkey,
    /// Seller's USDC token account — receives milestone releases
    pub seller_receive_token_account: Pubkey,
    /// Judge (Flamingo oracle backend) pubkey — seeds the vault PDA
    pub judge_key:                    Pubkey,
    /// Remaining USDC amount held in vault (decremented on each release)
    pub amount:                       u64,
    /// Unique order identifier
    pub order_code:                   u64,
    /// Current status of the escrow
    pub status:                       EscrowStatus,
    /// Unix timestamp when shipping() was confirmed by oracle
    pub shipped_time:                 i64,
    /// Unix timestamp when delivered() was confirmed by oracle
    pub delivery_time:                i64,
    /// Unix timestamp when raise_dispute() was called by buyer
    pub dispute_time:                 i64,
    /// Carrier used for shipping
    pub carrier:                      Carrier,
    /// Tracking ID string registered by oracle at shipping confirmation
    #[max_len(50)]
    pub tracking_id:                  String,
    /// Calculated platform fee (5% of initial amount)
    pub platform_fee:                 u64,
    /// Upfront logistics fee paid at deposit
    pub logistics_fee:                u64,
    /// Amount added to the circuit-breaker rolling volume at initialize.
    /// Used for consistent decrement on cancel/refund regardless of later
    /// partial releases (e.g., the 50% seller share released at shipping).
    pub deposited_amount:             u64,
}
