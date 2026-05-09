# Lambda Escrow Smart Contract — Technical Documentation

**Program ID:** `BcEopLQ9MxMdMtU57m5KYA4sk9qvhy29XkneEKHcfuSf`  
**Version:** 3.2 (Enhanced)  
**Framework:** Anchor 0.31.1  
**Network:** Solana (Devnet / Mainnet-Beta)  
**Token Standard:** SPL Token (Token Program v1)

---

## Overview

Lambda Escrow is a logistics-driven, SPL-token escrow protocol. It features an upfront logistics fee collection and enforces a **50/50 payment split** (of the remaining escrow amount after fees) triggered by verified shipping milestones. A platform-controlled `judge` (Oracle) acts as the exclusive authority for validating logistics data and releasing funds, while buyers retain the ability to raise disputes within a **configurable** window.

---

## Platform Fee & Logistics

The platform retains a **5% fee** on the total deposit, collected at **shipping**, and requires an upfront **logistics fee** which is transferred immediately upon deposit.

| Flow | Amount |
|------|--------|
| Buyer pays | Logistics Fee → Platform Vault<br>Remaining Deposit → Escrow Vault |
| Shipping | Platform Fee: 5% collected from Escrow Vault<br>Seller: 50% of the remaining vault balance |
| Delivery | Seller: 50% (Final balance) |

**Fee Collection:** Admin calls `collect_fees()` to transfer accumulated fees from platform vault to a designated account.

---

## Architecture: Privy & Solana Pay

The platform utilizes a hybrid authentication and payment model:
- **Privy MPC (Primary):** Users interact via non-custodial embedded wallets. All dashboard actions (initializing orders, raising disputes) are signed using Privy's MPC infrastructure.
- **Solana Pay (Mobile):** Authenticated users can scan a QR code to sign the `initialize` transaction using an external mobile wallet (e.g., Phantom Mobile).

---

## PDA Derivation

All PDAs are scoped to the `judge` public key and `order_code`:

```
vault_account       = PDA(["vault", judge.key(), order_code.to_le_bytes()])
vault_authority     = PDA(["authority", judge.key(), order_code.to_le_bytes()])
escrow_account      = PDA(["escrow", judge.key(), order_code.to_le_bytes()])
program_config      = PDA(["config"])
platform_fee_vault  = PDA(["platform_fee_vault"])
platform_fee_authority = PDA(["platform_fee_authority"])
```

## ProgramConfig State

```rust
pub struct ProgramConfig {
    pub admin: Pubkey,                   // Admin pubkey (can update config, collect fees)
    pub is_paused: bool,                 // Circuit breaker flag
    pub current_volume: u64,              // Rolling volume in window
    pub volume_threshold: u64,          // Threshold that triggers breaker
    pub last_volume_reset_time: i64,       // Last window reset timestamp
    pub window_duration: i64,           // Rolling window duration
    pub platform_fee_vault: Pubkey,      // Platform fee vault PDA
    pub accumulated_fees: u64,          // Accumulated platform fees
    pub dispute_window: i64,            // Dispute window in seconds
    pub dispute_resolution_deadline: i64, // Resolution deadline in seconds
}
```

## EscrowAccount State

```rust
pub enum EscrowStatus { Funded, Shipped, Delivered, Disputed, Released, Adjudicated, Refunded }
pub enum Carrier { Dhl, Aramex, Fedex, Sendbox }

pub struct EscrowAccount {
    pub buyer_key: Pubkey,                    // 32 bytes
    pub buyer_deposit_token_account: Pubkey,  // 32 bytes
    pub seller_key: Pubkey,                   // 32 bytes
    pub seller_receive_token_account: Pubkey, // 32 bytes
    pub judge_key: Pubkey,                    // 32 bytes
    pub amount: u64,                          // 8 bytes (remaining vault balance)
    pub order_code: u64,                      // 8 bytes
    pub status: EscrowStatus,                 // 1 byte (Serialized as index)
    pub shipped_time: i64,                    // 8 bytes
    pub delivery_time: i64,                   // 8 bytes
    pub dispute_time: i64,                    // 8 bytes
    pub carrier: Carrier,                     // 1 byte (Serialized as index)
    pub tracking_id: String,                  // 4 + max 50 bytes
    pub platform_fee: u64,                   // 8 bytes (Stored 5% fee calculated at init)
    pub logistics_fee: u64,                  // 8 bytes (Upfront logistics fee paid at deposit)
}
```

### Status Codes (Enums)

| Enum Variant | Meaning |
|---|---|
| **Funded** | Initialized; awaiting shipping confirmation. |
| **Shipped** | 50% of remaining balance released to seller; tracking registered. |
| **Delivered** | Item delivered; configurable dispute window active. |
| **Disputed** | Buyer raised a dispute; funds locked. |
| **Released** | Final 50% released via `exchange` (Success). |
| **Adjudicated** | Dispute resolved by Judge. |
| **Refunded** | Order cancelled or refunded. |

---

## Lifecycle Flow

```
[Buyer] initialize()          → status: Funded
[Judge] shipping()            → status: Shipped  (50% of remaining → seller)
[Judge] delivered()           → status: Delivered  (Configurable window starts)

  ┌── No dispute: [Judge] exchange() → status: Released (final 50% → seller)
  └── Dispute:    [Buyer] raise_dispute()    → status: Disputed
                  [Judge] adjudge()          → status: Adjudicated (funds → buyer or seller)
```

---

## Instructions

### `initialize_config`
Initializes the program configuration and creates the platform fee vault.
**Authorized Signer:** Admin.
**Parameters:** `volume_threshold`, `window_duration`, `dispute_window`, `dispute_resolution_deadline`.

### `update_config`
Updates program configuration.
**Authorized Signer:** Admin.
**Parameters:** `is_paused`, `volume_threshold` (optional), `window_duration` (optional), `dispute_window` (optional), `dispute_resolution_deadline` (optional).

### `collect_fees`
Collects accumulated platform fees to a destination account.
**Authorized Signer:** Admin.

### `initialize`
Creates a new escrow order.
**Authorized Signer:** Buyer.
**Action:** Performs token account validation (freeze checks, mint verification).

### `cancel` / `cancel_partial`
Cancels an escrow and returns funds to buyer.
**Authorized Signer:** Judge.
**Constraint:** Only callable when status is Funded.

### `shipping`
Triggered by the platform when a tracking ID is verified.
**Authorized Signer:** Judge (Oracle Backend).
**Parameters:** `order_code`, `tracking_id`, `carrier_code` (0-3 mapped to Enum).
**Action:** Collects platform fee and releases **50%** of the remaining funds to the seller.

### `delivered`
Triggered when the carrier marks the package as delivered.
**Authorized Signer:** Judge (Oracle Backend).
**Action:** Sets `delivery_time`, starting the configurable dispute clock.

### `raise_dispute`
**Authorized Signer:** Buyer.
**Constraint:** Must be called within `dispute_window` of `delivery_time`.
**Action:** Locks funds and awaits `adjudge`.

### `exchange`
Releases the remaining **50%** after the dispute window expires.
**Authorized Signer:** Judge (Platform Cron).

### `refund` / `refund_partial`
Full or partial refund to buyer when order is cancelled post-shipping.
**Authorized Signer:** Judge only.

### `adjudge`
Resolves a disputed order.
**Authorized Signer:** Judge.
**Constraint:** Must be called within `dispute_resolution_deadline`.

---

## Security & Access Control

1. **Judge-Centric Model:** The **Judge** is the only party authorized to move funds based on logistics milestones.
2. **Token Safety:** Instruction `initialize` verifies that token accounts are not frozen and use the correct mint.
3. **Safe Math:** All volume management and fee calculations use `checked_sub`, `checked_add`, etc. to prevent overflows.
4. **Circuit Breaker:** Automatically pauses the program if USDC volume exceeds the threshold.

---

## Error Codes

| Code | Message |
|---|---|
| `ProgramPaused` | Program is paused by circuit breaker |
| `MathOverflow` | Arithmetic overflow/underflow |
| `InvalidStatus` | Invalid status for operation |
| `DisputeWindowExpired` | Too late to raise/resolve a dispute |
| `AccountFrozen` | Token account is frozen |
| `InvalidMint` | Invalid mint for this operation |
| ... | (See `errors.rs` for full list) |

---

## Changelog

### v3.2 (Current - Enhanced)
- **Configurable Windows:** Added `dispute_window` and `dispute_resolution_deadline` to global config.
- **Enums:** Replaced magic number status codes and carrier codes with Rust `enum` types for better safety and readability.
- **Improved Validation:** Added frozen account checks and mint verification in `initialize`.
- **Safe Math:** Replaced all `saturating_sub` with `checked_sub` to handle underflows explicitly.
- **Event Types:** Created distinct event types for operation outcomes (`PartialRefundProcessed`, `DisputeResolved`).

### v3.1
- **Account Standardization:** Unified all client account naming to camelCase.
- **PDA Security Hardening:** Fixed seed collision potential and strictly enforced PDA constraints.
- **Full Fee Reversal:** `refund()` now returns the platform fee to the buyer if already collected.
- **State Integrity:** Added `platform_fee` to `EscrowAccount`.

### v3.0
- **Added Platform Fee:** 5% platform fee deducted at shipping.
- **Added Platform Vault:** New PDA for accumulating fees.
- **Added collect_fees:** Admin function to collect accumulated fees.
- **Added Events:** `FeesCollected` event.

### v2.0
- **Removed charge_more:** One-time checkout only
- **Security Fix:** Refund functions moved to Judge-only (seller/buyer cannot call)
- **Status Updates:** Added status updates in `adjudge()`, `refund()`, `exchange()`
- **Vault Validation:** Added vault balance checks in all fund release operations
- **cancel_partial:** Added vault balance validation and status check
- **Auto-Close:** RefundPartial now auto-closes when fully refunded
- **Events:** Added PartialRefund event for off-chain tracking
- **Cleanup:** Removed unused constants
