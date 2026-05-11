# Flamingo Live Escrow — Technical Documentation

**Program ID:** `Gp3Qy7yguTZmDkNwKUxpweQfcRNsQ5tRQTjEkjqcDgSV`  
**Version:** 3.4  
**Framework:** Anchor 0.31.1  
**Network:** Solana (Devnet / Mainnet-Beta)  
**Token Standard:** SPL Token (Token Program v1)

---

## Overview

Flamingo Live Escrow is a logistics-driven, SPL-token escrow protocol. It features an upfront logistics fee collection and enforces a **50/50 payment split** (of the remaining escrow amount after fees) triggered by verified shipping milestones. A platform-controlled `judge` (Oracle) acts as the exclusive authority for validating logistics data and releasing funds, while buyers retain the ability to raise disputes within a **configurable** window.

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
vault_account          = PDA(["vault", judge.key(), order_code.to_le_bytes()])
vault_authority        = PDA(["authority", judge.key(), order_code.to_le_bytes()])
escrow_account         = PDA(["escrow", judge.key(), order_code.to_le_bytes()])
program_config         = PDA(["config"])
platform_fee_vault     = PDA(["platform_fee_vault"])
platform_fee_authority = PDA(["platform_fee_authority"])
```

## ProgramConfig State

```rust
pub struct ProgramConfig {
    pub admin: Pubkey,                        // Admin pubkey (can update config, collect fees)
    pub is_paused: bool,                      // Circuit breaker flag
    pub current_volume: u64,                  // Rolling volume in window
    pub volume_threshold: u64,               // Threshold that triggers breaker
    pub last_volume_reset_time: i64,          // Last window reset timestamp
    pub window_duration: i64,                // Rolling window duration
    pub platform_fee_vault: Pubkey,           // Platform fee vault PDA
    pub accumulated_fees: u64,               // Accumulated platform fees
    pub dispute_window: i64,                 // Dispute window in seconds
    pub dispute_resolution_deadline: i64,    // Resolution deadline in seconds
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
    pub amount: u64,                          // 8 bytes  (current vault balance)
    pub order_code: u64,                      // 8 bytes
    pub status: EscrowStatus,                 // 1 byte
    pub shipped_time: i64,                    // 8 bytes
    pub delivery_time: i64,                   // 8 bytes
    pub dispute_time: i64,                    // 8 bytes
    pub carrier: Carrier,                     // 1 byte
    pub tracking_id: String,                  // 4 + max 50 bytes
    pub platform_fee: u64,                    // 8 bytes  (5% fee calculated at init)
    pub logistics_fee: u64,                   // 8 bytes  (upfront logistics fee)
    pub deposited_amount: u64,                // 8 bytes  (amount added to circuit-breaker rolling volume at init)
}
```

> **`deposited_amount`**: Stores the vault-held amount added to `current_volume` at `initialize`. Used for consistent circuit-breaker decrements on `cancel`/`refund` — ensures the rolling window is correctly unwound regardless of partial milestone releases (e.g. the 50% seller share released at `shipping`).

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
Updates program configuration. Use `is_paused: true` to manually trip the circuit breaker.
**Authorized Signer:** Admin.
**Parameters:** `is_paused`, `volume_threshold` (optional), `window_duration` (optional), `dispute_window` (optional), `dispute_resolution_deadline` (optional).

### `collect_fees`
Collects accumulated platform fees to a destination account.
**Authorized Signer:** Admin.

### `update_admin`
Transfers admin authority to a new public key. The current admin must sign.
**Authorized Signer:** Current Admin.
**Parameters:** `new_admin` — the public key of the incoming admin.
**Note:** Irreversible from the old key once confirmed. Store the new admin keypair securely before calling.

### `initialize`
Creates a new escrow order. Validates token accounts (frozen check, mint verification) before any transfers.
**Authorized Signer:** Buyer.

### `cancel` / `cancel_partial`
Cancels an escrow and returns funds to buyer. Closes the vault and escrow accounts.
**Authorized Signer:** Judge.
**Constraint:** Only callable when status is `Funded`.

### `shipping`
Triggered by the platform when a tracking ID is verified.
**Authorized Signer:** Judge (Oracle Backend).
**Parameters:** `order_code`, `tracking_id`, `carrier_code` (0–3).
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
**Authorized Signer:** Judge (Platform Cron). Closes vault and escrow accounts.

### `refund` / `refund_partial`
Full or partial refund to buyer. `refund` also returns the platform fee already collected and closes both vault and escrow accounts.
**Authorized Signer:** Judge only.
**Constraint:** Status must be `Shipped`, `Delivered`, or `Disputed`.

### `adjudge`
Resolves a disputed order. Closes vault and escrow accounts.
**Authorized Signer:** Judge.
**Parameters:** `status` — `0` = rule for buyer, `2` = rule for seller.
**Constraint:** Must be called within `dispute_resolution_deadline`.

---

## Security & Access Control

1. **Judge-Centric Model:** The **Judge** is the only party authorized to move funds based on logistics milestones.
2. **Token Safety:** `initialize` verifies token accounts are not frozen and use the correct mint — checked before any CPI transfers.
3. **Safe Math:** All volume management and fee calculations use `checked_sub`, `checked_add`, etc.
4. **Circuit Breaker:** Rejects deposits exceeding the rolling volume threshold; admin manually pauses via `update_config`.
5. **Consistent Volume Tracking:** `deposited_amount` ensures circuit-breaker decrements match original additions.

---

## Error Codes

| Code | Message |
|---|---|
| `ProgramPaused` | Program is paused by circuit breaker |
| `CircuitBreakerTripped` | Volume threshold exceeded |
| `MathOverflow` | Arithmetic overflow/underflow |
| `InvalidStatus` | Invalid status for operation |
| `DisputeWindowExpired` | Buyer's dispute window has expired — cannot raise a dispute after the configured window |
| `DisputeResolutionDeadlineExpired` | Judge's resolution deadline has passed — cannot call `adjudge` after `dispute_resolution_deadline` |
| `AccountFrozen` | Token account is frozen |
| `InvalidMint` | Invalid mint for this operation |
| `InvalidTrackingId` | Tracking ID must be 8–50 characters |
| `TrackingIdMismatch` | Tracking ID doesn't match registered ID |
| `InvalidCarrierCode` | Carrier code must be 0–3 |
| `InsufficientFunds` | Insufficient token balance |
| `InvalidOwner` | Token account owner mismatch |
| `InvalidOrder` | Order code mismatch |
| `InvalidAccount` | Invalid account provided |
| `InvalidAmount` | Amount must be greater than zero |
| `InvalidConfig` | Config parameter must be greater than zero |

---

## Changelog

### v3.4 (Current)
- **New Instruction:** `update_admin` — allows the current admin to transfer authority to a new public key.
- **New Error:** `DisputeResolutionDeadlineExpired` — distinct from `DisputeWindowExpired`; surfaces when the judge attempts to call `adjudge` after `dispute_resolution_deadline` has passed.
- **Bug Fix:** `adjudge` now correctly raises `DisputeResolutionDeadlineExpired` instead of the buyer-facing `DisputeWindowExpired`.
- **Bug Fix:** All judge instruction bodies (`adjudge`, `refund`, `refund_partial`) now derive PDA seeds from `escrow_account.judge_key` for consistency with their `#[account]` constraint seeds.
- **Bug Fix:** `refund_partial` replaced `saturating_sub` with `checked_sub` — underflows now return `MathOverflow` instead of silently producing zero.
- **Deploy:** Program re-deployed to devnet at new address `Gp3Qy7yguTZmDkNwKUxpweQfcRNsQ5tRQTjEkjqcDgSV` (previous `BcEopLQ9` deprecated).
- **Testing:** 19-test localnet suite added covering all payment flows, dispute paths, circuit breaker, and admin key rotation.

### v3.3
- **Rename:** Program renamed from `lambda-escrow` to `flamingolive-escrow`.
- **Bug Fix:** Token account validation (frozen/mint checks) moved before transfers in `initialize`.
- **Bug Fix:** Circuit breaker `is_paused = true` removed from failing transaction path (state was always rolled back); admin must use `update_config` to manually pause.
- **Bug Fix:** Added `deposited_amount` field to `EscrowAccount` for consistent circuit-breaker volume decrements across `cancel`, `refund`, and partial operations.
- **Bug Fix:** `Refund` instruction now closes the escrow account on completion (consistent with all other terminal instructions).
- **Bug Fix:** `refund()` now emits `EscrowRefunded` event (correct buyer-centric fields) instead of the misleading `FundsReleased` with a seller key.
- **Refactor:** Removed unused `platform_fee_vault` account from `Exchange` struct (fee already collected at `shipping`).
- **Cleanup:** Removed dead events `EscrowAdjudicated` and `PartialRefund`; added `EscrowRefunded`.

### v3.2
- **Configurable Windows:** Added `dispute_window` and `dispute_resolution_deadline` to global config.
- **Enums:** Replaced magic number status codes and carrier codes with Rust `enum` types.
- **Improved Validation:** Added frozen account checks and mint verification in `initialize`.
- **Safe Math:** Replaced all `saturating_sub` with `checked_sub` to handle underflows explicitly.
- **Event Types:** Created distinct event types for operation outcomes (`PartialRefundProcessed`, `DisputeResolved`).

### v3.1
- **Account Standardization:** Unified all client account naming to camelCase.
- **PDA Security Hardening:** Fixed seed collision potential and strictly enforced PDA constraints.
- **Full Fee Reversal:** `refund()` returns the platform fee to the buyer if already collected.
- **State Integrity:** Added `platform_fee` to `EscrowAccount`.

### v3.0
- **Added Platform Fee:** 5% platform fee deducted at shipping.
- **Added Platform Vault:** New PDA for accumulating fees.
- **Added collect_fees:** Admin function to collect accumulated fees.
- **Added Events:** `FeesCollected` event.

### v2.0
- **Removed charge_more:** One-time checkout only.
- **Security Fix:** Refund functions moved to Judge-only.
- **Status Updates:** Added status updates in `adjudge()`, `refund()`, `exchange()`.
- **Vault Validation:** Added vault balance checks in all fund release operations.
- **cancel_partial:** Added vault balance validation and status check.
- **Auto-Close:** `refund_partial` now auto-closes when fully refunded.
