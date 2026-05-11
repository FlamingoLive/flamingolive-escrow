# Flamingo Live Escrow — Frontend & Backend Integration Guide

This guide walks developers through integrating the Flamingo Live Escrow smart contract using **Privy MPC (Embedded Wallets)** for the dashboard and **Solana Pay** for authenticated mobile payments.

---

## 💰 Fees & Logistics

The contract enforces an upfront logistics fee collected at initialization, and a **5% platform fee** collected at shipping:

| Step | Action | Amount |
|------|--------|--------|
| 1 | Buyer pays to escrow | Logistics fee immediately to platform; remainder to escrow vault |
| 2 | Shipping: Platform fee | 5% (of initial total) to platform vault |
| 3 | Shipping: Seller receives | 50% of the remaining balance |
| 4 | Delivery: Seller receives | Final 50% of the remaining balance |

**Example:** Buyer escrows 100 USDC with a 10 USDC logistics fee
- Init: 10 USDC logistics fee to platform; 90 USDC to escrow vault
- Shipping: Platform gets 5 USDC (5% of 100); Seller gets 42.5 USDC (50% of 85)
- Delivery: Seller gets 42.5 USDC (Final 50%)

---

## Prerequisites

```bash
npm install @coral-xyz/anchor @solana/web3.js @solana/spl-token @privy-io/react-auth
```

---

## 1. Setup: Privy MPC (Embedded Wallet)

Privy provides a non-custodial platform wallet for all users. You do not need a standard wallet adapter.

### 1.1 Bridge Privy to Anchor
Create a `useAnchorProgram` hook to wrap the Privy wallet:

```typescript
import { useMemo } from 'react';
import { useWallets } from '@privy-io/react-auth/solana';
import { Program, AnchorProvider } from '@coral-xyz/anchor';
import { Connection, PublicKey } from '@solana/web3.js';
import idl from './idl.json';

export function useAnchorProgram() {
  const { wallets } = useWallets();
  const connection = new Connection("https://api.devnet.solana.com", "confirmed");

  return useMemo(() => {
    // Find the Privy embedded wallet
    const privyWallet = wallets.find((w) => w.walletClientType === 'privy');
    if (!privyWallet) return null;

    // Create Anchor-compatible wallet object
    const anchorWallet = {
      publicKey: new PublicKey(privyWallet.address),
      signTransaction: (tx) => privyWallet.signTransaction(tx),
      signAllTransactions: (txs) => privyWallet.signAllTransactions(txs),
    };

    const provider = new AnchorProvider(connection, anchorWallet, { commitment: 'confirmed' });
    return new Program(idl as any, provider);
  }, [wallets]);
}
```

---

## 2. Setup: Backend (Judge/Oracle)

The **Judge** (Flamingo Oracle) is a server-side process that signs with a dedicated private key.

```typescript
import * as anchor from "@coral-xyz/anchor";
import { Program, AnchorProvider } from "@coral-xyz/anchor";
import { Connection, PublicKey, Keypair } from "@solana/web3.js";

const connection = new Connection("https://api.devnet.solana.com", "confirmed");
const judgeKeypair = Keypair.fromSecretKey(Buffer.from(process.env.JUDGE_SECRET_KEY, "base64"));
const wallet = new anchor.Wallet(judgeKeypair);
const provider = new AnchorProvider(connection, wallet, { commitment: "confirmed" });
const program = new Program(idl as any, provider);
```

---

## 3. Solana Pay: Authenticated Mobile Flow

Use Solana Pay for authenticated users who prefer a mobile signing experience. This allows them to create an escrow via a QR code while logged in.

### 3.1 Backend: Transaction Request Endpoint
Your backend builds the transaction and returns it to the mobile wallet.

```typescript
// GET /api/solana-pay/escrow
// Returns metadata (Label, Icon)
res.json({ label: "Flamingo Escrow", icon: "https://flamingo.com/icon.png" });

// POST /api/solana-pay/escrow
// Receives the user's public key (account)
router.post("/escrow", async (req, res) => {
  const { account } = req.body; // The buyer's public key
  const { amount, orderCode, logisticsFee } = req.query;

  const buyerPubkey = new PublicKey(account);
  const [vaultPda] = getVaultPDA(judgePubkey, orderCode);
  const [vaultAuthorityPda] = getVaultAuthorityPDA(judgePubkey, orderCode);

  // Build the instruction
  const transaction = await program.methods
    .initialize(new anchor.BN(amount), new anchor.BN(orderCode), new anchor.BN(logisticsFee))
    .accounts({
      buyer: buyerPubkey,
      // ... other accounts ...
    })
    .transaction();

  transaction.feePayer = buyerPubkey;
  transaction.recentBlockhash = (await connection.getLatestBlockhash()).blockhash;

  const serialized = transaction.serialize({ requireAllSignatures: false });
  res.json({
    transaction: serialized.toString("base64"),
    message: `Secure ${amount} USDC in Escrow #${orderCode}`,
  });
});
```

---

## 4. Dashboard: Privy Interaction

Once the user is logged in, use the `useAnchorProgram` hook for all dashboard actions.

### 4.1 Raising a Dispute
```typescript
const program = useAnchorProgram();

async function handleDispute(orderCode: number) {
  if (!program) return;
  
  await program.methods
    .raiseDispute(new anchor.BN(orderCode))
    .accounts({
      buyer: program.provider.publicKey,
      escrowAccount: escrowAccountPubkey,
    })
    .rpc();
}
```

---

## 5. Summary: Actor & Signing Logic

| Milestone | Actor | Signing Method |
|---|---|---|
| **Escrow Creation** | Buyer | **Solana Pay** (QR) or **Privy** (In-app) |
| **Cancel** | **Judge** | **Backend** (Private Key) |
| **Shipping (50% of remaining)** | **Judge** | **Backend** (Private Key) |
| **Delivery (Trial Start)**| **Judge** | **Backend** (Private Key) |
| **Auto-Release (Final 50%)** | **Judge** | **Backend** (Private Key) |
| **Dispute** | Buyer | **Privy** (In-app) |
| **Refund** | **Judge** | **Backend** (Private Key) |
| **Resolution** | **Judge** | **Backend** (Private Key) |

---

## 6. PDA Utility (`pda.ts`)

```typescript
const PROGRAM_ID = new PublicKey("BcEopLQ9MxMdMtU57m5KYA4sk9qvhy29XkneEKHcfuSf");

export function getVaultPDA(judgeKey: PublicKey, orderCode: number) {
  const orderCodeBuffer = new anchor.BN(orderCode).toArrayLike(Buffer, "le", 8);
  return PublicKey.findProgramAddressSync([Buffer.from("vault"), judgeKey.toBuffer(), orderCodeBuffer], PROGRAM_ID);
}

export function getVaultAuthorityPDA(judgeKey: PublicKey, orderCode: number) {
  const orderCodeBuffer = new anchor.BN(orderCode).toArrayLike(Buffer, "le", 8);
  return PublicKey.findProgramAddressSync([Buffer.from("authority"), judgeKey.toBuffer(), orderCodeBuffer], PROGRAM_ID);
}

export function getEscrowPDA(judgeKey: PublicKey, orderCode: number) {
  const orderCodeBuffer = new anchor.BN(orderCode).toArrayLike(Buffer, "le", 8);
  return PublicKey.findProgramAddressSync([Buffer.from("escrow"), judgeKey.toBuffer(), orderCodeBuffer], PROGRAM_ID);
}

export function getPlatformFeeVaultPDA() {
  return PublicKey.findProgramAddressSync([Buffer.from("platform_fee_vault")], PROGRAM_ID);
}

export function getPlatformFeeAuthorityPDA() {
  return PublicKey.findProgramAddressSync([Buffer.from("platform_fee_authority")], PROGRAM_ID);
}

export function getConfigPDA() {
  return PublicKey.findProgramAddressSync([Buffer.from("config")], PROGRAM_ID);
}
```
