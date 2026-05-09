import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { LambdaEscrow } from "../target/types/lambda_escrow";
import { 
  TOKEN_PROGRAM_ID, 
  Token 
} from "@solana/spl-token";
import { 
  PublicKey, 
  SystemProgram, 
  Keypair, 
  LAMPORTS_PER_SOL 
} from "@solana/web3.js";
import { assert } from "chai";

describe("lambda-escrow", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.LambdaEscrow as Program<LambdaEscrow>;
  
  let mintA: Token;
  let buyerTokenAccountA: PublicKey;
  let sellerTokenAccountA: PublicKey;

  const buyer = Keypair.generate();
  const seller = Keypair.generate();
  const judge = Keypair.generate(); // Oracle
  
  const amount = 1000;
  let orderCodeCounter = 1000;

  function getNextOrderCode() {
    return orderCodeCounter++;
  }

  // PDAs
  let config_pda: PublicKey;
  let platform_vault_pda: PublicKey;
  let platform_authority_pda: PublicKey;

  before(async () => {
    // Airdrop SOL
    await provider.connection.confirmTransaction(
      await provider.connection.requestAirdrop(buyer.publicKey, 10 * LAMPORTS_PER_SOL)
    );
    await provider.connection.confirmTransaction(
      await provider.connection.requestAirdrop(judge.publicKey, 10 * LAMPORTS_PER_SOL)
    );
    await provider.connection.confirmTransaction(
      await provider.connection.requestAirdrop(provider.wallet.publicKey, 10 * LAMPORTS_PER_SOL)
    );

    mintA = await Token.createMint(
      provider.connection,
      buyer,
      buyer.publicKey,
      null,
      0,
      TOKEN_PROGRAM_ID
    );

    buyerTokenAccountA = await mintA.createAccount(buyer.publicKey);
    sellerTokenAccountA = await mintA.createAccount(seller.publicKey);

    await mintA.mintTo(buyerTokenAccountA, buyer, [], amount + 10000);

    [config_pda] = PublicKey.findProgramAddressSync([Buffer.from("config")], program.programId);
    [platform_vault_pda] = PublicKey.findProgramAddressSync([Buffer.from("platform_fee_vault")], program.programId);
    [platform_authority_pda] = PublicKey.findProgramAddressSync([Buffer.from("platform_fee_authority")], program.programId);
  });

  async function derivePDAs(orderCode: number) {
    const orderBytes = new anchor.BN(orderCode).toArrayLike(Buffer, "le", 8);
    const [vault_account_pda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), judge.publicKey.toBuffer(), orderBytes],
      program.programId
    );
    const [vault_authority_pda] = PublicKey.findProgramAddressSync(
      [Buffer.from("authority"), judge.publicKey.toBuffer(), orderBytes],
      program.programId
    );
    const [escrow_pda] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow"), judge.publicKey.toBuffer(), orderBytes],
      program.programId
    );
    return { vault_account_pda, vault_authority_pda, escrow_pda };
  }

  it("Initialize config", async () => {
    await program.methods.initializeConfig(
      new anchor.BN(1000000), // volume_threshold
      new anchor.BN(3600),    // window_duration
      new anchor.BN(86400),   // dispute_window
      new anchor.BN(259200)   // dispute_resolution_deadline
    )
      .accounts({
        admin: provider.wallet.publicKey,
        mint: mintA.publicKey,
        platformFeeVault: platform_vault_pda,
        platformFeeVaultAuthority: platform_authority_pda,
        config: config_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .rpc();
  });

  it("Full Escrow Lifecycle (Init -> Shipping -> Delivered -> Exchange)", async () => {
    const orderCode = getNextOrderCode();
    const { vault_account_pda, vault_authority_pda, escrow_pda } = await derivePDAs(orderCode);
    const logisticsFee = 100;

    // 1. Initialize
    await program.methods.initialize(new anchor.BN(amount), new anchor.BN(orderCode), new anchor.BN(logisticsFee))
      .accounts({
        buyer: buyer.publicKey,
        seller: seller.publicKey,
        judge: judge.publicKey,
        mint: mintA.publicKey,
        buyerDepositTokenAccount: buyerTokenAccountA,
        sellerReceiveTokenAccount: sellerTokenAccountA,
        escrowAccount: escrow_pda,
        config: config_pda,
        vaultAccount: vault_account_pda,
        vaultAuthority: vault_authority_pda,
        platformFeeVault: platform_vault_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([buyer])
      .rpc();

    let escrow = await program.account.escrowAccount.fetch(escrow_pda);
    assert.deepEqual(escrow.status, { funded: {} });
    assert.strictEqual(escrow.amount.toNumber(), amount - logisticsFee);

    // 2. Shipping
    await program.methods.shipping(new anchor.BN(orderCode), "TRACKING_123", 0) // 0 = DHL
      .accounts({
        buyer: buyer.publicKey,
        buyerDepositTokenAccount: buyerTokenAccountA,
        seller: seller.publicKey,
        sellerReceiveTokenAccount: sellerTokenAccountA,
        judge: judge.publicKey,
        escrowAccount: escrow_pda,
        vaultAccount: vault_account_pda,
        vaultAuthority: vault_authority_pda,
        config: config_pda,
        platformFeeVault: platform_vault_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([judge])
      .rpc();

    escrow = await program.account.escrowAccount.fetch(escrow_pda);
    assert.deepEqual(escrow.status, { shipped: {} });
    assert.deepEqual(escrow.carrier, { dhl: {} });

    // 3. Delivered
    await program.methods.delivered(new anchor.BN(orderCode), "TRACKING_123")
      .accounts({
        config: config_pda,
        judge: judge.publicKey,
        buyer: buyer.publicKey,
        seller: seller.publicKey,
        escrowAccount: escrow_pda,
      })
      .signers([judge])
      .rpc();

    escrow = await program.account.escrowAccount.fetch(escrow_pda);
    assert.deepEqual(escrow.status, { delivered: {} });

    // 4. Exchange
    // Update config to have 0 window for testing
    await program.methods.updateConfig(false, null, null, new anchor.BN(1), null)
      .accounts({
        admin: provider.wallet.publicKey,
        config: config_pda,
      })
      .rpc();

    // Wait for the 1s window to pass
    await new Promise(resolve => setTimeout(resolve, 2000));

    await program.methods.exchange()
      .accounts({
        judge: judge.publicKey,
        buyer: buyer.publicKey,
        seller: seller.publicKey,
        sellerReceiveTokenAccount: sellerTokenAccountA,
        escrowAccount: escrow_pda,
        vaultAccount: vault_account_pda,
        vaultAuthority: vault_authority_pda,
        config: config_pda,
        platformFeeVault: platform_vault_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([judge])
      .rpc();

    try {
      await program.account.escrowAccount.fetch(escrow_pda);
      assert.fail("Escrow account should be closed");
    } catch (e) {}
  });

  it("Escrow Cancellation Flow", async () => {
    const orderCode = getNextOrderCode();
    const { vault_account_pda, vault_authority_pda, escrow_pda } = await derivePDAs(orderCode);
    const logisticsFee = 50;

    await program.methods.initialize(new anchor.BN(amount), new anchor.BN(orderCode), new anchor.BN(logisticsFee))
      .accounts({
        buyer: buyer.publicKey,
        seller: seller.publicKey,
        judge: judge.publicKey,
        mint: mintA.publicKey,
        buyerDepositTokenAccount: buyerTokenAccountA,
        sellerReceiveTokenAccount: sellerTokenAccountA,
        escrowAccount: escrow_pda,
        config: config_pda,
        vaultAccount: vault_account_pda,
        vaultAuthority: vault_authority_pda,
        platformFeeVault: platform_vault_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([buyer])
      .rpc();

    await program.methods.cancel(new anchor.BN(orderCode))
      .accounts({
        config: config_pda,
        judge: judge.publicKey,
        buyer: buyer.publicKey,
        buyerDepositTokenAccount: buyerTokenAccountA,
        escrowAccount: escrow_pda,
        vaultAccount: vault_account_pda,
        vaultAuthority: vault_authority_pda,
        platformFeeVault: platform_vault_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([judge])
      .rpc();

    try {
      await program.account.escrowAccount.fetch(escrow_pda);
      assert.fail("Escrow account should be closed");
    } catch (e) {}
  });

  it("Escrow Refund Flow (Post-Shipping)", async () => {
    const orderCode = getNextOrderCode();
    const { vault_account_pda, vault_authority_pda, escrow_pda } = await derivePDAs(orderCode);
    const logisticsFee = 20;

    await program.methods.initialize(new anchor.BN(amount), new anchor.BN(orderCode), new anchor.BN(logisticsFee))
      .accounts({
        buyer: buyer.publicKey,
        seller: seller.publicKey,
        judge: judge.publicKey,
        mint: mintA.publicKey,
        buyerDepositTokenAccount: buyerTokenAccountA,
        sellerReceiveTokenAccount: sellerTokenAccountA,
        escrowAccount: escrow_pda,
        config: config_pda,
        vaultAccount: vault_account_pda,
        vaultAuthority: vault_authority_pda,
        platformFeeVault: platform_vault_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([buyer])
      .rpc();

    await program.methods.shipping(new anchor.BN(orderCode), "TRACKING_REFUND", 1) // Aramex
      .accounts({
        buyer: buyer.publicKey,
        buyerDepositTokenAccount: buyerTokenAccountA,
        seller: seller.publicKey,
        sellerReceiveTokenAccount: sellerTokenAccountA,
        judge: judge.publicKey,
        escrowAccount: escrow_pda,
        vaultAccount: vault_account_pda,
        vaultAuthority: vault_authority_pda,
        config: config_pda,
        platformFeeVault: platform_vault_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([judge])
      .rpc();

    // Judge decides to refund remaining balance to buyer
    await program.methods.refund(new anchor.BN(orderCode))
      .accounts({
        config: config_pda,
        judge: judge.publicKey,
        buyer: buyer.publicKey,
        buyerDepositTokenAccount: buyerTokenAccountA,
        seller: seller.publicKey,
        sellerReceiveTokenAccount: sellerTokenAccountA,
        escrowAccount: escrow_pda,
        vaultAccount: vault_account_pda,
        vaultAuthority: vault_authority_pda,
        platformFeeVault: platform_vault_pda,
        platformFeeVaultAuthority: platform_authority_pda,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([judge])
      .rpc();

    try {
      await program.account.escrowAccount.fetch(escrow_pda);
      assert.fail("Escrow account should be closed");
    } catch (e) {}
  });

});
