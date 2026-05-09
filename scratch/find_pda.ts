import * as anchor from "@coral-xyz/anchor";
import { PublicKey } from "@solana/web3.js";

const programId = new PublicKey("CXWCr2nFZ5yXuewf5t2GFYTT337XmaH8UrhUbS2Hy8tL");
const [configPDA] = PublicKey.findProgramAddressSync(
  [Buffer.from("config")],
  programId
);

console.log("Config PDA:", configPDA.toBase58());
