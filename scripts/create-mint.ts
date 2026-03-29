import "dotenv/config";

import { readFileSync } from "node:fs";

import { createMint } from "@solana/spl-token";
import { Connection, Keypair, PublicKey } from "@solana/web3.js";

function requireEnv(name: string): string {
    const value = process.env[name];
    if (!value) {
        throw new Error(`Missing required env var: ${name}`);
    }
    return value;
}

function getAdminKeypair(): Keypair {
    const keypairPath = requireEnv("ADMIN_KEYPAIR_PATH");
    const secret = JSON.parse(readFileSync(keypairPath, "utf8")) as number[];
    return Keypair.fromSecretKey(Uint8Array.from(secret));
}

async function main(): Promise<void> {
    const rpcUrl = requireEnv("RPC_URL");
    const authority = getAdminKeypair();
    const decimals = 6;

    const connection = new Connection(rpcUrl, "confirmed");
    const mint = await createMint(
        connection,
        authority,
        authority.publicKey,
        null,
        decimals,
    );

    console.log(`RPC URL: ${rpcUrl}`);
    console.log(`Authority: ${authority.publicKey.toBase58()}`);
    console.log(`Decimals: ${decimals}`);
    console.log(`Mint: ${mint.toBase58()}`);
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
