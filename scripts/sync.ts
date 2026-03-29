import "dotenv/config";

import { readFileSync } from "node:fs";

import { getAssociatedTokenAddressSync } from "@solana/spl-token";
import {
    sendAndConfirmTransaction,
    Connection,
    Keypair,
    PublicKey,
    Transaction,
    TransactionInstruction,
} from "@solana/web3.js";

const FACTORY_SEED = "factory";
const PAIR_SEED_PREFIX = "pair";
const TOKEN_PROGRAM_ID = new PublicKey(
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const ATA_PROGRAM_ID = new PublicKey(
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);
const CLOCK_SYSVAR_ID = new PublicKey(
    "SysvarC1ock11111111111111111111111111111111",
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

function deriveFactoryPda(programId: PublicKey): [PublicKey, number] {
    return PublicKey.findProgramAddressSync([Buffer.from(FACTORY_SEED)], programId);
}

function derivePairPda(
    programId: PublicKey,
    factory: PublicKey,
    mint0: PublicKey,
    mint1: PublicKey,
): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
        [
            Buffer.from(PAIR_SEED_PREFIX),
            factory.toBuffer(),
            mint0.toBuffer(),
            mint1.toBuffer(),
        ],
        programId,
    );
}

function getPdaAta(owner: PublicKey, mint: PublicKey): PublicKey {
    return getAssociatedTokenAddressSync(
        mint,
        owner,
        true,
        TOKEN_PROGRAM_ID,
        ATA_PROGRAM_ID,
    );
}

function canonicalMints(
    mintA: PublicKey,
    mintB: PublicKey,
): [PublicKey, PublicKey] {
    return Buffer.compare(mintA.toBuffer(), mintB.toBuffer()) < 0
        ? [mintA, mintB]
        : [mintB, mintA];
}

function readU64LE(data: Buffer, offset: number): bigint {
    return data.readBigUInt64LE(offset);
}

function createSyncInstruction(args: {
    programId: PublicKey;
    pair: PublicKey;
    vault0: PublicKey;
    vault1: PublicKey;
}): TransactionInstruction {
    return new TransactionInstruction({
        programId: args.programId,
        keys: [
            { pubkey: args.pair, isSigner: false, isWritable: true },
            { pubkey: args.vault0, isSigner: false, isWritable: false },
            { pubkey: args.vault1, isSigner: false, isWritable: false },
            { pubkey: CLOCK_SYSVAR_ID, isSigner: false, isWritable: false },
        ],
        data: Buffer.from([6]),
    });
}

async function main(): Promise<void> {
    const rpcUrl = requireEnv("RPC_URL");
    const programId = new PublicKey(requireEnv("PROGRAM_ID"));
    const payer = getKeypair("USER_KEYPAIR_PATH");
    const mintA = new PublicKey(requireEnv("MINT_A"));
    const mintB = new PublicKey(requireEnv("MINT_B"));

    const connection = new Connection(rpcUrl, "confirmed");
    const [factory] = deriveFactoryPda(programId);
    const [mint0, mint1] = canonicalMints(mintA, mintB);
    const [pair] = derivePairPda(programId, factory, mint0, mint1);
    const vault0 = getPdaAta(pair, mint0);
    const vault1 = getPdaAta(pair, mint1);

    const beforePair = await connection.getAccountInfo(pair, "confirmed");
    if (!beforePair) {
        throw new Error("Pair PDA does not exist.");
    }

    const reserve0Before = readU64LE(beforePair.data, 193);
    const reserve1Before = readU64LE(beforePair.data, 201);
    const vault0Balance = BigInt((await connection.getTokenAccountBalance(vault0, "confirmed")).value.amount);
    const vault1Balance = BigInt((await connection.getTokenAccountBalance(vault1, "confirmed")).value.amount);

    const ix = createSyncInstruction({
        programId,
        pair,
        vault0,
        vault1,
    });

    const tx = new Transaction().add(ix);
    const signature = await sendAndConfirmTransaction(connection, tx, [payer]);

    const afterPair = await connection.getAccountInfo(pair, "confirmed");
    if (!afterPair) {
        throw new Error("Pair PDA disappeared after sync.");
    }

    const reserve0After = readU64LE(afterPair.data, 193);
    const reserve1After = readU64LE(afterPair.data, 201);

    console.log(`RPC URL: ${rpcUrl}`);
    console.log(`Program ID: ${programId.toBase58()}`);
    console.log(`Pair PDA: ${pair.toBase58()}`);
    console.log(`Vault0 ATA: ${vault0.toBase58()}`);
    console.log(`Vault1 ATA: ${vault1.toBase58()}`);
    console.log(`Vault0 balance: ${vault0Balance.toString()}`);
    console.log(`Vault1 balance: ${vault1Balance.toString()}`);
    console.log(`Reserve0 before: ${reserve0Before.toString()}`);
    console.log(`Reserve1 before: ${reserve1Before.toString()}`);
    console.log(`Reserve0 after: ${reserve0After.toString()}`);
    console.log(`Reserve1 after: ${reserve1After.toString()}`);
    console.log(`Sync signature: ${signature}`);
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
