# 🦩 Flamingo Live — Escrow

[![Solana](https://img.shields.io/badge/Solana-Devnet-black?logo=solana)](https://solana.com)
[![Anchor Framework](https://img.shields.io/badge/Anchor-v0.31.1-blue)](https://project-serum.github.io/anchor/)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Version](https://img.shields.io/badge/Version-3.4-green)]()

**Program ID**: `Gp3Qy7yguTZmDkNwKUxpweQfcRNsQ5tRQTjEkjqcDgSV` (Devnet)

**Flamingo Live Escrow** is a Solana smart contract purpose-built for the Flamingo Live marketplace. It facilitates secure global commerce by connecting African sellers with international buyers through a logistics-verified escrow system.

Funds are held in USDC and released based on real-world delivery milestones verified by a trusted logistics oracle (The Judge).

---

## 💰 Fees & Logistics

The platform retains a **5% fee** on every transaction. The logistics fee is paid upfront when the buyer funds the escrow.

- **Buyer pays**: Full amount (Total Deposit + Logistics Fee)
- **Funding (Initialization)**:  
  - Logistics fee is immediately released to the platform vault to cover carrier costs.
  - Platform fee (5%) is calculated based on the deposit.
- **Shipping (Milestone 1)**:  
  - Platform collects the **5%** fee.
  - Seller receives **50%** of the remaining escrow amount for liquidity.
- **Delivery (Milestone 2)**: 
  - Remaining **50%** to seller after the configurable dispute window expires.

---

## 💸 Core Workflow: The 50/50 Split

The escrow implements a logistics-driven release schedule to balance seller cash flow with buyer protection:

1.  **Funded (0%):** Buyer deposits full USDC amount. The logistics fee is immediately paid to the platform to secure shipping.
2.  **Shipped (50%):** Once the **carrier** provides a valid tracking ID, the platform fee is collected and **50%** of the remaining escrow funds are released to the seller to ensure they remain liquid.
3.  **Delivered (50%):** When the carrier marks the package as delivered, the **dispute window** opens.
4.  **Completed:** If no dispute is raised within the configured window, the final **50%** is released to the seller.

---

## 🛡️ Key Features

### ⚖️ Trusted Logistics Oracle (The Judge)

The Flamingo platform acts as the "Judge." Only the Judge keypair can confirm shipping and delivery milestones. This centralized trust model ensures that funds are only released when physical logistics scans are verified against carriers (DHL, Aramex, FedEx, Sendbox).

### ⏱️ Configurable Dispute Window

Once delivery is confirmed, buyers have a configurable window to raise a dispute (default: 24 hours). The admin can adjust this via the update_config instruction to ensure fairness and prevent infinite fund locking.

### 🔌 Circuit Breaker Volume Management

To protect against flash-loan attacks or platform exploits, the contract includes a rolling volume circuit breaker. If the total USDC volume exceeds a configurable threshold within a specific window, new deposits are rejected. The admin can pause/unpause manually via `update_config`.

### 📦 Multi-Carrier Support

The contract supports and validates carrier codes for major logistics providers:

- `0`: DHL
- `1`: Aramex
- `2`: FedEx
- `3`: Sendbox

---

## 🔌 Architecture: Privy & Solana Pay

The platform uses a modern, non-custodial architecture to ensure a seamless user experience while maintaining decentralization:

- **Privy MPC (Embedded Wallets):** All users are assigned a platform wallet via Privy. This allows for an "in-app" dashboard experience where users can sign transactions (like raising disputes) without needing external browser extensions.
- **Solana Pay (Logged-in Mobile Flow):** For authenticated users who prefer a mobile experience, the platform supports Solana Pay transaction requests. This allows a logged-in buyer to scan a QR code and sign the escrow initialization directly from their mobile wallet (Phantom/Solflare).

---

## 👥 Role Definitions

| Role       | Responsibility                                                                                       |
| :--------- | :--------------------------------------------------------------------------------------------------- |
| **Buyer**  | Deposits USDC, raises disputes after delivery.                                                         |
| **Seller**  | Ships goods, receives 50% on shipping, 50% on delivery completion.                                |
| **Judge**  | The logistics oracle / platform. Authorized to call `shipping()`, `delivered()`, `exchange()`, `adjudge()`, `cancel()`, `refund()`, and `refund_partial()`. |
| **Admin** | Platform admin. Authorized to call `initialize_config()`, `update_config()`, `collect_fees()`, and `update_admin()`. |

---

## 🛠️ Development Setup

### Prerequisites

- **Rust**: `1.75.0+`
- **Solana CLI**: `1.18.0+`
- **Anchor**: `0.31.1`
- **Node.js**: `18.x+`

### Installation

```bash
# Clone the repository
git clone https://github.com/FlamingoLive/flamingolive-escrow.git
cd flamingolive-escrow

# Install dependencies
npm install
```

### Build & Test

```bash
# Build the program
anchor build

# Run the full test suite (19 tests, spins up local validator automatically)
anchor test

# Deploy to Devnet
anchor deploy --provider.cluster devnet
```

---

## 📁 Project Structure

The project follows a modularized Anchor structure for better maintainability and security auditing:

```text
programs/flamingolive-escrow/src/
├── constants.rs      # Default values (e.g., 24h window)
├── errors.rs         # Custom ErrorCodes
├── events.rs         # All on-chain events for off-chain indexing
├── instructions/     # Logic partitioned by role (buyer, judge, admin, logistics)
├── state/            # Account structures (EscrowAccount, ProgramConfig)
└── lib.rs            # Program entry point
```

---

## 🔒 Security & Oracle Integration

The Judge keypair is the most critical component of the security model. In production, this key should be stored in a **Hardware Security Module (HSM)** or a cloud KMS (AWS KMS / GCP KMS).

The contract validates:

- **Tracking ID Length**: Minimum 8 characters to prevent fake IDs.
- **Circuit Breaker**: Consistent volume tracking via `deposited_amount` field — decrements on cancel/refund regardless of partial milestone releases.
- **Token Safety**: Frozen account and mint checks run before any transfers.
- **Status Locks**: Refunds are blocked during active disputes.

---

## 📜 License

This project is licensed under the MIT License.
