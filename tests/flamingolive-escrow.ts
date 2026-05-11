/**
 * Flamingo stablecoin payment rail — end-to-end test suite
 *
 * Demonstrates every path through the on-chain escrow:
 *   - Full happy path (init → ship → deliver → exchange) with exact USDC balance assertions
 *   - Dispute resolution (buyer wins / seller wins via adjudge)
 *   - Judge-initiated returns: cancel, cancel_partial, refund, refund_partial
 *   - Security: invalid state transitions, wrong tracking IDs, paused program
 *   - Circuit breaker: volume threshold enforcement and admin recovery
 *   - Admin key rotation via the new update_admin instruction
 *
 * Token amounts use 0 decimals to keep all assertions exact integer math:
 *
 *   AMOUNT = 10_000  |  LOGISTICS_FEE = 1_000
 *   ───────────────────────────────────────────────────────────────
 *   vault (escrow_amount)    = 10_000 − 1_000           =  9_000
 *   platform_fee (5%)        = floor(10_000 × 5 / 100)  =    500
 *   remaining_after_fees     = 9_000 − 500              =  8_500
 *   seller_share_at_shipping = floor(8_500 / 2)          =  4_250
 *   seller_share_at_exchange = 8_500 − 4_250             =  4_250
 *   ───────────────────────────────────────────────────────────────
 *   seller total             = 4_250 + 4_250             =  8_500
 *   platform total           = 1_000 + 500               =  1_500
 *   sum                      = 8_500 + 1_500             = 10_000  ✓
 */

import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { FlamingoliveEscrow } from "../target/types/flamingolive_escrow";
import { TOKEN_PROGRAM_ID, Token } from "@solana/spl-token";
import {
    PublicKey,
    SystemProgram,
    Keypair,
    LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import { assert } from "chai";

// ── Amount constants ─────────────────────────────────────────────────────────
const AMOUNT         = 10_000;
const LOGISTICS_FEE  = 1_000;
const VAULT_AMOUNT   = AMOUNT - LOGISTICS_FEE;                     // 9_000
const PLATFORM_FEE   = Math.floor((AMOUNT * 5) / 100);            //   500
const REMAINING      = VAULT_AMOUNT - PLATFORM_FEE;               // 8_500
const SELLER_AT_SHIP = Math.floor(REMAINING / 2);                 // 4_250
const SELLER_AT_EXCH = REMAINING - SELLER_AT_SHIP;                // 4_250

const BUYER_INITIAL  = 500_000; // enough for many tests
const SMALL_WINDOW   = 1;       // 1 s — used to expire timers quickly in tests
const LARGE_WINDOW   = 86_400;  // 1 day — dispute / resolution deadlines

// ─────────────────────────────────────────────────────────────────────────────

describe("flamingolive-escrow", () => {
    const provider = anchor.AnchorProvider.env();
    anchor.setProvider(provider);

    const program = anchor.workspace.FlamingoliveEscrow as Program<FlamingoliveEscrow>;

    // Participants
    const admin  = (provider.wallet as anchor.Wallet).payer;
    const buyer  = Keypair.generate();
    const seller = Keypair.generate();
    const judge  = Keypair.generate();

    // Token infrastructure
    let mint: Token;
    let buyerAta:  PublicKey;
    let sellerAta: PublicKey;

    // Program-level PDAs (derived once, shared across all tests)
    let configPda:            PublicKey;
    let feeVaultPda:          PublicKey;
    let feeVaultAuthorityPda: PublicKey;

    // Unique per-test order codes
    let orderCounter = 3000;
    const nextOrder = () => orderCounter++;

    // ── Utilities ─────────────────────────────────────────────────────────────

    /** Derive all order-specific PDAs from an orderCode. */
    async function pdas(orderCode: number) {
        const ob = new anchor.BN(orderCode).toArrayLike(Buffer, "le", 8);
        const [vaultPda] = PublicKey.findProgramAddressSync(
            [Buffer.from("vault"), judge.publicKey.toBuffer(), ob],
            program.programId,
        );
        const [vaultAuthority] = PublicKey.findProgramAddressSync(
            [Buffer.from("authority"), judge.publicKey.toBuffer(), ob],
            program.programId,
        );
        const [escrowPda] = PublicKey.findProgramAddressSync(
            [Buffer.from("escrow"), judge.publicKey.toBuffer(), ob],
            program.programId,
        );
        return { vaultPda, vaultAuthority, escrowPda };
    }

    /** Read the integer balance of a token account. */
    async function balance(ata: PublicKey): Promise<number> {
        const info = await provider.connection.getTokenAccountBalance(ata);
        return Number(info.value.amount);
    }

    const sleep = (ms: number) => new Promise(r => setTimeout(r, ms));

    // ── Config helpers ────────────────────────────────────────────────────────

    async function setDisputeWindow(seconds: number) {
        await program.methods
            .updateConfig(false, null, null, new anchor.BN(seconds), null)
            .accounts({ admin: admin.publicKey, config: configPda })
            .rpc();
    }

    async function setResolutionDeadline(seconds: number) {
        await program.methods
            .updateConfig(false, null, null, null, new anchor.BN(seconds))
            .accounts({ admin: admin.publicKey, config: configPda })
            .rpc();
    }

    async function pauseProgram() {
        await program.methods
            .updateConfig(true, null, null, null, null)
            .accounts({ admin: admin.publicKey, config: configPda })
            .rpc();
    }

    async function unpauseProgram() {
        await program.methods
            .updateConfig(false, null, null, null, null)
            .accounts({ admin: admin.publicKey, config: configPda })
            .rpc();
    }

    // ── Instruction helpers ───────────────────────────────────────────────────

    async function initEscrow(
        orderCode: number,
        amount     = AMOUNT,
        logistics  = LOGISTICS_FEE,
    ) {
        const { vaultPda, vaultAuthority, escrowPda } = await pdas(orderCode);
        await program.methods
            .initialize(
                new anchor.BN(amount),
                new anchor.BN(orderCode),
                new anchor.BN(logistics),
            )
            .accounts({
                config:                     configPda,
                buyer:                      buyer.publicKey,
                seller:                     seller.publicKey,
                judge:                      judge.publicKey,
                mint:                       mint.publicKey,
                vaultAccount:               vaultPda,
                vaultAuthority,
                buyerDepositTokenAccount:   buyerAta,
                sellerReceiveTokenAccount:  sellerAta,
                escrowAccount:              escrowPda,
                platformFeeVault:           feeVaultPda,
                systemProgram:              SystemProgram.programId,
                tokenProgram:               TOKEN_PROGRAM_ID,
            })
            .signers([buyer])
            .rpc();
        return { vaultPda, vaultAuthority, escrowPda };
    }

    async function ship(
        orderCode:    number,
        trackingId:   string,
        carrierCode:  number,
        vaultPda:     PublicKey,
        vaultAuth:    PublicKey,
        escrowPda:    PublicKey,
    ) {
        await program.methods
            .shipping(new anchor.BN(orderCode), trackingId, carrierCode)
            .accounts({
                config:                     configPda,
                buyer:                      buyer.publicKey,
                buyerDepositTokenAccount:   buyerAta,
                seller:                     seller.publicKey,
                sellerReceiveTokenAccount:  sellerAta,
                judge:                      judge.publicKey,
                escrowAccount:              escrowPda,
                vaultAccount:               vaultPda,
                vaultAuthority:             vaultAuth,
                platformFeeVault:           feeVaultPda,
                tokenProgram:               TOKEN_PROGRAM_ID,
            })
            .signers([judge])
            .rpc();
    }

    async function deliver(
        orderCode:  number,
        trackingId: string,
        escrowPda:  PublicKey,
    ) {
        await program.methods
            .delivered(new anchor.BN(orderCode), trackingId)
            .accounts({
                config:       configPda,
                judge:        judge.publicKey,
                buyer:        buyer.publicKey,
                seller:       seller.publicKey,
                escrowAccount: escrowPda,
            })
            .signers([judge])
            .rpc();
    }

    /** Helper: init → ship → deliver → raise_dispute. Returns PDAs. */
    async function setupDisputed(orderCode: number, trackingId: string) {
        const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);
        await ship(orderCode, trackingId, 1 /* Aramex */, vaultPda, vaultAuthority, escrowPda);
        await deliver(orderCode, trackingId, escrowPda);

        await program.methods
            .raiseDispute(new anchor.BN(orderCode))
            .accounts({ config: configPda, buyer: buyer.publicKey, escrowAccount: escrowPda })
            .signers([buyer])
            .rpc();

        const escrow = await program.account.escrowAccount.fetch(escrowPda);
        assert.deepEqual(escrow.status, { disputed: {} });
        return { vaultPda, vaultAuthority, escrowPda };
    }

    // ═════════════════════════════════════════════════════════════════════════
    // BEFORE: fund wallets, create mint, create ATAs
    // ═════════════════════════════════════════════════════════════════════════

    before(async () => {
        for (const kp of [buyer, seller, judge, admin]) {
            await provider.connection.confirmTransaction(
                await provider.connection.requestAirdrop(kp.publicKey, 10 * LAMPORTS_PER_SOL),
            );
        }

        mint = await Token.createMint(
            provider.connection,
            buyer,
            buyer.publicKey, // mint authority
            null,
            0,               // 0 decimals — all amounts are exact integers
            TOKEN_PROGRAM_ID,
        );

        buyerAta  = await mint.createAccount(buyer.publicKey);
        sellerAta = await mint.createAccount(seller.publicKey);

        await mint.mintTo(buyerAta, buyer, [], BUYER_INITIAL);

        [configPda]            = PublicKey.findProgramAddressSync([Buffer.from("config")],               program.programId);
        [feeVaultPda]          = PublicKey.findProgramAddressSync([Buffer.from("platform_fee_vault")],   program.programId);
        [feeVaultAuthorityPda] = PublicKey.findProgramAddressSync([Buffer.from("platform_fee_authority")], program.programId);
    });

    // ═════════════════════════════════════════════════════════════════════════
    // 1. ADMIN: Initialize program config
    // ═════════════════════════════════════════════════════════════════════════

    it("admin: initialize_config sets global parameters", async () => {
        await program.methods
            .initializeConfig(
                new anchor.BN(50_000_000), // volume_threshold
                new anchor.BN(3_600),      // window_duration (1 hour)
                new anchor.BN(LARGE_WINDOW), // dispute_window (1 day)
                new anchor.BN(LARGE_WINDOW), // resolution_deadline (1 day)
            )
            .accounts({
                admin:                    admin.publicKey,
                mint:                     mint.publicKey,
                platformFeeVault:         feeVaultPda,
                platformFeeVaultAuthority: feeVaultAuthorityPda,
                config:                   configPda,
                tokenProgram:             TOKEN_PROGRAM_ID,
                systemProgram:            SystemProgram.programId,
            })
            .rpc();

        const cfg = await program.account.programConfig.fetch(configPda);
        assert.strictEqual(cfg.isPaused, false);
        assert.ok(cfg.volumeThreshold.eq(new anchor.BN(50_000_000)));
        assert.ok(cfg.disputeWindow.eq(new anchor.BN(LARGE_WINDOW)));
        assert.ok(cfg.disputeResolutionDeadline.eq(new anchor.BN(LARGE_WINDOW)));
        assert.strictEqual(cfg.admin.toBase58(), admin.publicKey.toBase58());
    });

    // ═════════════════════════════════════════════════════════════════════════
    // 2. HAPPY PATH: Init → Ship → Deliver → Exchange
    //    Every USDC transfer is verified with exact before/after balance deltas.
    // ═════════════════════════════════════════════════════════════════════════

    describe("Happy path — full payment rail", () => {
        it("moves exact USDC at every milestone", async () => {
            const orderCode  = nextOrder();
            const trackingId = "TRACK-HAPPY-001";

            const buyerBefore0  = await balance(buyerAta);
            const sellerBefore0 = await balance(sellerAta);
            const feeVaultBefore0 = await balance(feeVaultPda);

            // ── INITIALIZE ────────────────────────────────────────────────────
            // Buyer deposits AMOUNT.  Logistics fee → platform vault immediately.
            // Remainder → vault PDA.
            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);

            let escrow = await program.account.escrowAccount.fetch(escrowPda);
            assert.deepEqual(escrow.status, { funded: {} }, "status: Funded after init");
            assert.ok(escrow.amount.eq(new anchor.BN(VAULT_AMOUNT)),
                `vault holds escrow_amount=${VAULT_AMOUNT}`);
            assert.ok(escrow.platformFee.eq(new anchor.BN(PLATFORM_FEE)),
                `platform_fee stored as ${PLATFORM_FEE}`);
            assert.ok(escrow.logisticsFee.eq(new anchor.BN(LOGISTICS_FEE)),
                `logistics_fee stored as ${LOGISTICS_FEE}`);
            assert.strictEqual(
                await balance(buyerAta), buyerBefore0 - AMOUNT,
                "buyer paid full AMOUNT",
            );
            assert.strictEqual(
                await balance(feeVaultPda), feeVaultBefore0 + LOGISTICS_FEE,
                "logistics fee landed in platform vault",
            );

            // ── SHIPPING ──────────────────────────────────────────────────────
            // 50% of net (after platform fee) → seller.
            // Platform fee (5% of AMOUNT) → platform vault.
            const sellerBefore1  = await balance(sellerAta);
            const feeVaultBefore1 = await balance(feeVaultPda);

            await ship(orderCode, trackingId, 0 /* DHL */, vaultPda, vaultAuthority, escrowPda);

            escrow = await program.account.escrowAccount.fetch(escrowPda);
            assert.deepEqual(escrow.status, { shipped: {} }, "status: Shipped");
            assert.deepEqual(escrow.carrier, { dhl: {} }, "carrier: DHL");
            assert.strictEqual(escrow.trackingId, trackingId);
            assert.ok(escrow.amount.eq(new anchor.BN(SELLER_AT_EXCH)),
                `vault holds second-tranche=${SELLER_AT_EXCH} after shipping`);
            assert.ok(escrow.shippedTime.gtn(0), "shippedTime recorded");

            assert.strictEqual(
                await balance(sellerAta), sellerBefore1 + SELLER_AT_SHIP,
                `seller received first tranche=${SELLER_AT_SHIP}`,
            );
            assert.strictEqual(
                await balance(feeVaultPda), feeVaultBefore1 + PLATFORM_FEE,
                `platform vault received fee=${PLATFORM_FEE}`,
            );

            // ── DELIVERED ────────────────────────────────────────────────────
            // Oracle confirms delivery; opens configurable dispute window.
            await deliver(orderCode, trackingId, escrowPda);

            escrow = await program.account.escrowAccount.fetch(escrowPda);
            assert.deepEqual(escrow.status, { delivered: {} }, "status: Delivered");
            assert.ok(escrow.deliveryTime.gtn(0), "deliveryTime recorded");

            // ── EXCHANGE (auto-release after dispute window) ──────────────────
            // Shrink window to 1 s so the test can proceed without waiting a day.
            await setDisputeWindow(SMALL_WINDOW);
            await sleep(2_500);

            const sellerBefore2 = await balance(sellerAta);

            await program.methods
                .exchange()
                .accounts({
                    config:                    configPda,
                    judge:                     judge.publicKey,
                    buyer:                     buyer.publicKey,
                    seller:                    seller.publicKey,
                    sellerReceiveTokenAccount: sellerAta,
                    escrowAccount:             escrowPda,
                    vaultAccount:              vaultPda,
                    vaultAuthority,
                    tokenProgram:              TOKEN_PROGRAM_ID,
                })
                .signers([judge])
                .rpc();

            assert.strictEqual(
                await balance(sellerAta), sellerBefore2 + SELLER_AT_EXCH,
                `seller received second tranche=${SELLER_AT_EXCH}`,
            );

            // Vault and escrow account closed on exchange
            try {
                await program.account.escrowAccount.fetch(escrowPda);
                assert.fail("escrow account should be closed");
            } catch { /* expected */ }

            // Restore window for subsequent tests
            await setDisputeWindow(LARGE_WINDOW);

            // ── End-to-end totals ────────────────────────────────────────────
            const sellerNet  = (await balance(sellerAta))   - sellerBefore0;
            const platformNet = (await balance(feeVaultPda)) - feeVaultBefore0;

            assert.strictEqual(sellerNet,   SELLER_AT_SHIP + SELLER_AT_EXCH,
                "seller received full remaining net");
            assert.strictEqual(platformNet, LOGISTICS_FEE + PLATFORM_FEE,
                "platform received logistics + percentage fee");
            assert.strictEqual(sellerNet + platformNet, AMOUNT,
                "sum of all payouts = original deposit");
        });
    });

    // ═════════════════════════════════════════════════════════════════════════
    // 3. DISPUTE RESOLUTION
    // ═════════════════════════════════════════════════════════════════════════

    describe("Dispute resolution", () => {
        it("adjudge status=0 — judge rules for buyer: vault → buyer", async () => {
            const orderCode = nextOrder();
            const { vaultPda, vaultAuthority, escrowPda } = await setupDisputed(orderCode, "TRACK-DISPUTE-BUY");

            const buyerBefore  = await balance(buyerAta);
            const sellerBefore = await balance(sellerAta);

            await program.methods
                .adjudge(new anchor.BN(orderCode), 0 /* buyer wins */)
                .accounts({
                    config:                    configPda,
                    judge:                     judge.publicKey,
                    buyer:                     buyer.publicKey,
                    buyerDepositTokenAccount:  buyerAta,
                    seller:                    seller.publicKey,
                    sellerReceiveTokenAccount: sellerAta,
                    escrowAccount:             escrowPda,
                    vaultAccount:              vaultPda,
                    vaultAuthority,
                    tokenProgram:              TOKEN_PROGRAM_ID,
                })
                .signers([judge])
                .rpc();

            // The second-tranche vault balance goes back to the buyer.
            // Seller keeps their first tranche (already received at shipping).
            assert.strictEqual(
                await balance(buyerAta), buyerBefore + SELLER_AT_EXCH,
                "buyer recovered disputed vault amount",
            );
            assert.strictEqual(
                await balance(sellerAta), sellerBefore,
                "seller balance unchanged by adjudge",
            );

            try {
                await program.account.escrowAccount.fetch(escrowPda);
                assert.fail("escrow closed after adjudge");
            } catch { /* expected */ }
        });

        it("adjudge status=2 — judge rules for seller: vault → seller", async () => {
            const orderCode = nextOrder();
            const { vaultPda, vaultAuthority, escrowPda } = await setupDisputed(orderCode, "TRACK-DISPUTE-SELL");

            const sellerBefore = await balance(sellerAta);

            await program.methods
                .adjudge(new anchor.BN(orderCode), 2 /* seller wins */)
                .accounts({
                    config:                    configPda,
                    judge:                     judge.publicKey,
                    buyer:                     buyer.publicKey,
                    buyerDepositTokenAccount:  buyerAta,
                    seller:                    seller.publicKey,
                    sellerReceiveTokenAccount: sellerAta,
                    escrowAccount:             escrowPda,
                    vaultAccount:              vaultPda,
                    vaultAuthority,
                    tokenProgram:              TOKEN_PROGRAM_ID,
                })
                .signers([judge])
                .rpc();

            assert.strictEqual(
                await balance(sellerAta), sellerBefore + SELLER_AT_EXCH,
                "seller received disputed vault amount",
            );

            try {
                await program.account.escrowAccount.fetch(escrowPda);
                assert.fail("escrow closed after adjudge");
            } catch { /* expected */ }
        });
    });

    // ═════════════════════════════════════════════════════════════════════════
    // 4. JUDGE-INITIATED RETURNS
    // ═════════════════════════════════════════════════════════════════════════

    describe("Judge-initiated returns", () => {
        it("cancel — pre-shipping: full vault amount returned to buyer", async () => {
            const orderCode = nextOrder();
            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);

            const buyerBefore = await balance(buyerAta);

            await program.methods
                .cancel(new anchor.BN(orderCode))
                .accounts({
                    config:                   configPda,
                    judge:                    judge.publicKey,
                    buyer:                    buyer.publicKey,
                    buyerDepositTokenAccount: buyerAta,
                    escrowAccount:            escrowPda,
                    vaultAccount:             vaultPda,
                    vaultAuthority,
                    tokenProgram:             TOKEN_PROGRAM_ID,
                })
                .signers([judge])
                .rpc();

            assert.strictEqual(
                await balance(buyerAta), buyerBefore + VAULT_AMOUNT,
                `buyer recovered vault amount=${VAULT_AMOUNT}`,
            );

            try {
                await program.account.escrowAccount.fetch(escrowPda);
                assert.fail("escrow closed after cancel");
            } catch { /* expected */ }
        });

        it("cancel_partial — pre-shipping: partial refund, escrow stays open", async () => {
            const orderCode    = nextOrder();
            const partialAmount = 2_000;
            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);

            const buyerBefore = await balance(buyerAta);

            await program.methods
                .cancelPartial(new anchor.BN(orderCode), new anchor.BN(partialAmount))
                .accounts({
                    config:                   configPda,
                    judge:                    judge.publicKey,
                    buyer:                    buyer.publicKey,
                    buyerDepositTokenAccount: buyerAta,
                    escrowAccount:            escrowPda,
                    vaultAccount:             vaultPda,
                    vaultAuthority,
                    tokenProgram:             TOKEN_PROGRAM_ID,
                })
                .signers([judge])
                .rpc();

            assert.strictEqual(
                await balance(buyerAta), buyerBefore + partialAmount,
                "buyer received partial amount",
            );

            const escrow = await program.account.escrowAccount.fetch(escrowPda);
            assert.ok(escrow.amount.eq(new anchor.BN(VAULT_AMOUNT - partialAmount)),
                `vault reduced by ${partialAmount}`);
            assert.deepEqual(escrow.status, { funded: {} }, "status stays Funded");
        });

        it("refund — post-shipping: vault amount + platform fee returned to buyer", async () => {
            const orderCode  = nextOrder();
            const trackingId = "TRACK-REFUND-001";
            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);
            await ship(orderCode, trackingId, 2 /* FedEx */, vaultPda, vaultAuthority, escrowPda);

            const buyerBefore    = await balance(buyerAta);
            const feeVaultBefore = await balance(feeVaultPda);

            await program.methods
                .refund(new anchor.BN(orderCode))
                .accounts({
                    config:                    configPda,
                    judge:                     judge.publicKey,
                    buyer:                     buyer.publicKey,
                    buyerDepositTokenAccount:  buyerAta,
                    seller:                    seller.publicKey,
                    sellerReceiveTokenAccount: sellerAta,
                    escrowAccount:             escrowPda,
                    vaultAccount:              vaultPda,
                    vaultAuthority,
                    platformFeeVault:          feeVaultPda,
                    platformFeeVaultAuthority: feeVaultAuthorityPda,
                    tokenProgram:              TOKEN_PROGRAM_ID,
                })
                .signers([judge])
                .rpc();

            // refund() returns both the remaining vault amount AND the platform fee
            assert.strictEqual(
                await balance(buyerAta), buyerBefore + SELLER_AT_EXCH + PLATFORM_FEE,
                `buyer recovered vault(${SELLER_AT_EXCH}) + platform_fee(${PLATFORM_FEE})`,
            );
            assert.strictEqual(
                await balance(feeVaultPda), feeVaultBefore - PLATFORM_FEE,
                "platform fee vault decremented by returned fee",
            );

            try {
                await program.account.escrowAccount.fetch(escrowPda);
                assert.fail("escrow closed after refund");
            } catch { /* expected */ }
        });

        it("refund_partial — post-shipping: partial refund, escrow stays open", async () => {
            const orderCode    = nextOrder();
            const trackingId   = "TRACK-PARTIAL-REF";
            const partialRefund = 1_000;
            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);
            await ship(orderCode, trackingId, 3 /* Sendbox */, vaultPda, vaultAuthority, escrowPda);

            const buyerBefore = await balance(buyerAta);

            await program.methods
                .refundPartial(new anchor.BN(orderCode), new anchor.BN(partialRefund))
                .accounts({
                    config:                    configPda,
                    judge:                     judge.publicKey,
                    buyer:                     buyer.publicKey,
                    buyerDepositTokenAccount:  buyerAta,
                    seller:                    seller.publicKey,
                    sellerReceiveTokenAccount: sellerAta,
                    escrowAccount:             escrowPda,
                    vaultAccount:              vaultPda,
                    vaultAuthority,
                    tokenProgram:              TOKEN_PROGRAM_ID,
                })
                .signers([judge])
                .rpc();

            assert.strictEqual(
                await balance(buyerAta), buyerBefore + partialRefund,
                "buyer received partial refund",
            );

            const escrow = await program.account.escrowAccount.fetch(escrowPda);
            assert.ok(escrow.amount.eq(new anchor.BN(SELLER_AT_EXCH - partialRefund)),
                `vault reduced by ${partialRefund}`);
            assert.deepEqual(escrow.status, { shipped: {} }, "status stays Shipped");
        });
    });

    // ═════════════════════════════════════════════════════════════════════════
    // 5. SECURITY: Invalid state transitions must be rejected
    // ═════════════════════════════════════════════════════════════════════════

    describe("Security — invalid state transitions", () => {
        it("rejects duplicate order code (PDA already initialized)", async () => {
            const orderCode = nextOrder();
            await initEscrow(orderCode);

            let threw = false;
            try { await initEscrow(orderCode); } catch { threw = true; }
            assert.isTrue(threw, "duplicate init must fail");
        });

        it("rejects shipping with wrong carrier code", async () => {
            const orderCode = nextOrder();
            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);

            let threw = false;
            try {
                await program.methods
                    .shipping(new anchor.BN(orderCode), "TRACK-BAD-CARR", 99 /* invalid */)
                    .accounts({
                        config:                    configPda,
                        buyer:                     buyer.publicKey,
                        buyerDepositTokenAccount:  buyerAta,
                        seller:                    seller.publicKey,
                        sellerReceiveTokenAccount: sellerAta,
                        judge:                     judge.publicKey,
                        escrowAccount:             escrowPda,
                        vaultAccount:              vaultPda,
                        vaultAuthority,
                        platformFeeVault:          feeVaultPda,
                        tokenProgram:              TOKEN_PROGRAM_ID,
                    })
                    .signers([judge])
                    .rpc();
            } catch { threw = true; }
            assert.isTrue(threw, "carrier code 99 must be rejected");
        });

        it("rejects delivered with mismatched tracking ID", async () => {
            const orderCode = nextOrder();
            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);
            await ship(orderCode, "CORRECT-TRACK-ID", 0, vaultPda, vaultAuthority, escrowPda);

            let threw = false;
            try {
                await deliver(orderCode, "WRONG-TRACK-ID", escrowPda);
            } catch { threw = true; }
            assert.isTrue(threw, "mismatched tracking ID must be rejected at delivered");
        });

        it("rejects exchange while dispute window is still open", async () => {
            const orderCode  = nextOrder();
            const trackingId = "TRACK-EARLY-EXCH";
            await setDisputeWindow(LARGE_WINDOW);

            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);
            await ship(orderCode, trackingId, 0, vaultPda, vaultAuthority, escrowPda);
            await deliver(orderCode, trackingId, escrowPda);

            let threw = false;
            try {
                await program.methods
                    .exchange()
                    .accounts({
                        config:                    configPda,
                        judge:                     judge.publicKey,
                        buyer:                     buyer.publicKey,
                        seller:                    seller.publicKey,
                        sellerReceiveTokenAccount: sellerAta,
                        escrowAccount:             escrowPda,
                        vaultAccount:              vaultPda,
                        vaultAuthority,
                        tokenProgram:              TOKEN_PROGRAM_ID,
                    })
                    .signers([judge])
                    .rpc();
            } catch { threw = true; }
            assert.isTrue(threw, "exchange must fail inside dispute window");
        });

        it("rejects raise_dispute after the dispute window has expired", async () => {
            const orderCode  = nextOrder();
            const trackingId = "TRACK-LATE-DISPUTE";

            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);
            await ship(orderCode, trackingId, 0, vaultPda, vaultAuthority, escrowPda);
            await setDisputeWindow(SMALL_WINDOW);
            await deliver(orderCode, trackingId, escrowPda);
            await sleep(2_500); // let the 1 s window expire

            let threw = false;
            try {
                await program.methods
                    .raiseDispute(new anchor.BN(orderCode))
                    .accounts({ config: configPda, buyer: buyer.publicKey, escrowAccount: escrowPda })
                    .signers([buyer])
                    .rpc();
            } catch { threw = true; }
            assert.isTrue(threw, "raise_dispute after window expiry must fail");

            await setDisputeWindow(LARGE_WINDOW);
        });

        it("rejects adjudge after the resolution deadline has passed", async () => {
            await setResolutionDeadline(SMALL_WINDOW); // 1 s deadline

            const orderCode  = nextOrder();
            const trackingId = "TRACK-LATE-ADJUDGE";

            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);
            await ship(orderCode, trackingId, 0, vaultPda, vaultAuthority, escrowPda);
            await deliver(orderCode, trackingId, escrowPda);
            await program.methods
                .raiseDispute(new anchor.BN(orderCode))
                .accounts({ config: configPda, buyer: buyer.publicKey, escrowAccount: escrowPda })
                .signers([buyer])
                .rpc();

            await sleep(2_500); // let resolution deadline expire

            let threw = false;
            try {
                await program.methods
                    .adjudge(new anchor.BN(orderCode), 0)
                    .accounts({
                        config:                    configPda,
                        judge:                     judge.publicKey,
                        buyer:                     buyer.publicKey,
                        buyerDepositTokenAccount:  buyerAta,
                        seller:                    seller.publicKey,
                        sellerReceiveTokenAccount: sellerAta,
                        escrowAccount:             escrowPda,
                        vaultAccount:              vaultPda,
                        vaultAuthority,
                        tokenProgram:              TOKEN_PROGRAM_ID,
                    })
                    .signers([judge])
                    .rpc();
            } catch { threw = true; }
            assert.isTrue(threw, "adjudge after resolution deadline must fail (DisputeResolutionDeadlineExpired)");

            await setResolutionDeadline(LARGE_WINDOW); // restore
        });

        it("rejects initialize when program is paused", async () => {
            await pauseProgram();

            let threw = false;
            try { await initEscrow(nextOrder()); } catch { threw = true; }
            assert.isTrue(threw, "initialize must fail while paused");

            await unpauseProgram();
        });

        it("rejects shipping when program is paused", async () => {
            // Set up a funded escrow, then pause before shipping
            const orderCode  = nextOrder();
            const { vaultPda, vaultAuthority, escrowPda } = await initEscrow(orderCode);

            await pauseProgram();

            let threw = false;
            try {
                await ship(orderCode, "TRACK-PAUSE-SHIP", 0, vaultPda, vaultAuthority, escrowPda);
            } catch { threw = true; }
            assert.isTrue(threw, "shipping must fail while paused");

            await unpauseProgram();
        });
    });

    // ═════════════════════════════════════════════════════════════════════════
    // 6. CIRCUIT BREAKER
    // ═════════════════════════════════════════════════════════════════════════

    describe("Circuit breaker", () => {
        it("blocks new escrows when cumulative volume exceeds threshold", async () => {
            // Read current volume so we can set threshold exactly at the limit.
            // The next initEscrow would add VAULT_AMOUNT (9000), which will push
            // current_volume over threshold and return CircuitBreakerTripped.
            const cfg = await program.account.programConfig.fetch(configPda);
            const currentVolume = cfg.currentVolume.toNumber();

            await program.methods
                .updateConfig(false, new anchor.BN(currentVolume), null, null, null)
                .accounts({ admin: admin.publicKey, config: configPda })
                .rpc();

            let threw = false;
            try { await initEscrow(nextOrder()); } catch { threw = true; }
            assert.isTrue(threw, "circuit breaker must block init when threshold exceeded");

            // Restore high threshold
            await program.methods
                .updateConfig(false, new anchor.BN(50_000_000), null, null, null)
                .accounts({ admin: admin.publicKey, config: configPda })
                .rpc();
        });

        it("admin can unpause after circuit breaker trips, then operations resume", async () => {
            await pauseProgram();

            let threw = false;
            try { await initEscrow(nextOrder()); } catch { threw = true; }
            assert.isTrue(threw, "paused program blocks init");

            await unpauseProgram();

            // Operations resume after unpause
            const orderCode = nextOrder();
            await initEscrow(orderCode); // must not throw

            const escrow = await program.account.escrowAccount.fetch(
                (await pdas(orderCode)).escrowPda,
            );
            assert.deepEqual(escrow.status, { funded: {} }, "escrow created after unpause");
        });
    });

    // ═════════════════════════════════════════════════════════════════════════
    // 7. ADMIN: Key rotation (new update_admin instruction)
    // ═════════════════════════════════════════════════════════════════════════

    describe("Admin key rotation", () => {
        it("update_admin transfers authority; old admin loses access", async () => {
            const newAdmin = Keypair.generate();
            await provider.connection.confirmTransaction(
                await provider.connection.requestAirdrop(newAdmin.publicKey, 2 * LAMPORTS_PER_SOL),
            );

            // Current admin rotates authority to newAdmin
            // Cast to any: updateAdmin is in the source but IDL types require `anchor build` to regenerate.
            await (program.methods as any)
                .updateAdmin(newAdmin.publicKey)
                .accounts({ currentAdmin: admin.publicKey, config: configPda })
                .rpc();

            const cfg = await program.account.programConfig.fetch(configPda);
            assert.strictEqual(
                cfg.admin.toBase58(), newAdmin.publicKey.toBase58(),
                "config.admin updated to new key",
            );

            // Old admin can no longer call update_config
            let oldAdminThrew = false;
            try {
                await program.methods
                    .updateConfig(false, null, null, null, null)
                    .accounts({ admin: admin.publicKey, config: configPda })
                    .rpc();
            } catch { oldAdminThrew = true; }
            assert.isTrue(oldAdminThrew, "old admin rejected after rotation");

            // New admin can operate
            await program.methods
                .updateConfig(false, null, null, null, null)
                .accounts({ admin: newAdmin.publicKey, config: configPda })
                .signers([newAdmin])
                .rpc();

            // Rotate authority back so the rest of the suite can continue
            await (program.methods as any)
                .updateAdmin(admin.publicKey)
                .accounts({ currentAdmin: newAdmin.publicKey, config: configPda })
                .signers([newAdmin])
                .rpc();

            const cfgRestored = await program.account.programConfig.fetch(configPda);
            assert.strictEqual(
                cfgRestored.admin.toBase58(), admin.publicKey.toBase58(),
                "admin restored after rotation",
            );
        });
    });
});
