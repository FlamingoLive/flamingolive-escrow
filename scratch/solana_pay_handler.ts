import { Connection, PublicKey, Transaction } from "@solana/web3.js";
import * as anchor from "@coral-xyz/anchor";
import { getVaultPDA, getVaultAuthorityPDA } from "./pda_utils";

/**
 * Sample Express.js handler for a Solana Pay Transaction Request.
 * This builds an 'initialize' (Escrow Creation) transaction on the backend.
 */
export async function handleSolanaPayTransaction(req: any, res: any, program: anchor.Program) {
  try {
    const { account } = req.body; // The buyer's public key from the wallet
    const { amount, orderCode, judgeKey } = req.query;

    if (!account) {
      return res.status(400).json({ error: "Missing account (buyer public key)" });
    }

    const buyerPubkey = new PublicKey(account);
    const judgePubkey = new PublicKey(judgeKey);
    const orderCodeBN = new anchor.BN(orderCode);

    // 1. Derive necessary PDAs
    const [vaultPda] = getVaultPDA(judgePubkey, orderCode);
    const [vaultAuthorityPda] = getVaultAuthorityPDA(judgePubkey, orderCode);
    
    // Note: In a real app, you'd fetch the mint and config PDAs too
    const usdcMint = new PublicKey("..."); 
    const configPda = new PublicKey("...");
    const escrowAccount = anchor.web3.Keypair.generate();

    // 2. Build the instruction using Anchor's .transaction() method
    const transaction = await program.methods
      .initialize(new anchor.BN(amount), orderCodeBN)
      .accounts({
        config: configPda,
        buyer: buyerPubkey,
        seller: new PublicKey("..."), // Fetch from your DB
        judge: judgePubkey,
        mint: usdcMint,
        vaultAccount: vaultPda,
        vaultAuthority: vaultAuthorityPda,
        buyerDepositTokenAccount: new PublicKey("..."),
        sellerReceiveTokenAccount: new PublicKey("..."),
        escrowAccount: escrowAccount.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
      })
      .transaction();

    // 3. Prepare the transaction for the wallet
    transaction.feePayer = buyerPubkey;
    transaction.recentBlockhash = (await program.provider.connection.getLatestBlockhash()).blockhash;
    
    // The escrow account needs to sign for its creation
    transaction.partialSign(escrowAccount);

    // 4. Serialize to base64
    const serializedTransaction = transaction.serialize({
      requireAllSignatures: false, // User will sign next
    });

    res.status(200).json({
      transaction: serializedTransaction.toString("base64"),
      message: `Authorize ${amount} USDC for Escrow #${orderCode}`,
    });

  } catch (error) {
    console.error("Solana Pay Error:", error);
    res.status(500).json({ error: "Internal Server Error" });
  }
}
