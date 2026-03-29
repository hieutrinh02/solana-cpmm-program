import "dotenv/config";

import { readFileSync } from "node:fs";

import {
    createAssociatedTokenAccountIdempotent,
    getAssociatedTokenAddressSync,
    mintTo,
} from "@solana/spl-token";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";

const TOKEN_PROGRAM_ID = new PublicKey(
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);

function requireEnv(name: string): string {
    const value = process.env[name];
    if (!value) {
        throw new Error(`Missing required env var: ${name}`);
    }
    return value;
}

function getKeypair(pathEnv: string): Keypair {
    const keypairPath = requireEnv(pathEnv);
    const secret = JSON.parse(readFileSync(keypairPath, "utf8")) as number[];
    return Keypair.fromSecretKey(Uint8Array.from(secret));
}

async function main(): Promise<void> {
    const rpcUrl = requireEnv("RPC_URL");
    const authority = getKeypair("ADMIN_KEYPAIR_PATH");
    const mint = new PublicKey(requireEnv("MINT_A"));
    const recipient = authority.publicKey;
    const mintDecimals = 6;
    const amount = BigInt(1000 * (10 ** mintDecimals));

    const connection = new Connection(rpcUrl, "confirmed");
    const recipientAta = getAssociatedTokenAddressSync(
        mint,
        recipient,
        false,
        TOKEN_PROGRAM_ID,
    );

    await createAssociatedTokenAccountIdempotent(
        connection,
        authority,
        mint,
        recipient,
    );

    const signature = await mintTo(
        connection,
        authority,
        mint,
        recipientAta,
        authority,
        amount,
    );

    console.log(`RPC URL: ${rpcUrl}`);
    console.log(`Mint: ${mint.toBase58()}`);
    console.log(`Recipient: ${recipient.toBase58()}`);
    console.log(`Recipient ATA: ${recipientAta.toBase58()}`);
    console.log(`Amount: ${amount.toString()}`);
    console.log(`MintTo signature: ${signature}`);
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
