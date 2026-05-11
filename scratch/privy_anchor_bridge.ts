import { useMemo } from 'react';
import { useWallets } from '@privy-io/react-auth/solana';
import { Program, AnchorProvider } from '@coral-xyz/anchor';
import { Connection, PublicKey } from '@solana/web3.js';

/**
 * A React hook that bridges a Privy MPC embedded wallet to an Anchor Program.
 * This eliminates the need for standard Solana Wallet Adapters.
 * 
 * @param idl - The Anchor IDL for the flamingolive-escrow program
 * @param programId - The public key of the deployed program
 * @param rpcUrl - The Solana RPC URL (e.g. devnet)
 */
export function useFlamingoliveEscrowProgram(idl: any, programId: string, rpcUrl: string) {
  const { wallets } = useWallets();
  const connection = new Connection(rpcUrl, "confirmed");

  return useMemo(() => {
    // 1. Find the Privy embedded wallet among the user's connected wallets
    const privyWallet = wallets.find((w) => w.walletClientType === 'privy');
    if (!privyWallet) return null;

    // 2. Create an Anchor-compatible Wallet object using Privy's signing methods
    const anchorWallet = {
      publicKey: new PublicKey(privyWallet.address),
      signTransaction: async (tx: any) => {
        // Privy handles the MPC signing ceremony under the hood
        return await privyWallet.signTransaction(tx);
      },
      signAllTransactions: async (txs: any[]) => {
        return await privyWallet.signAllTransactions(txs);
      },
    };

    // 3. Instantiate the Anchor Provider and Program
    const provider = new AnchorProvider(connection, anchorWallet, {
      commitment: 'confirmed',
    });

    return new Program(idl, new PublicKey(programId), provider);
  }, [wallets, idl, programId, rpcUrl]);
}

/**
 * Example Usage in a Component:
 * 
 * const program = useFlamingoliveEscrowProgram(IDL, "DvDexLUC8x4ViabE5a3fhaPmRKLLJEPgW7iqMWBcjg11", "https://api.devnet.solana.com");
 * 
 * const createEscrow = async () => {
 *   if (!program) return;
 *   await program.methods.initialize(new BN(100), new BN(1234)).rpc();
 * };
 */
