import "dotenv/config";

import { readFileSync } from "node:fs";

import { getAssociatedTokenAddressSync } from "@solana/spl-token";
import {
    sendAndConfirmTransaction,
    Connection,
    Keypair,
    PublicKey,
    SystemProgram,
    Transaction,
    TransactionInstruction,
} from "@solana/web3.js";

const FACTORY_SEED = "factory";
const PAIR_SEED_PREFIX = "pair";
const LP_MINT_SEED_PREFIX = "lp_mint";
const TOKEN_PROGRAM_ID = new PublicKey(
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const ATA_PROGRAM_ID = new PublicKey(
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);
const RENT_SYSVAR_ID = new PublicKey(
    "SysvarRent111111111111111111111111111111111",
);
const PROGRAM_ADMIN_AUTHORITY = new PublicKey(
    "BNToqmqXLNvUrEGGS7io3MQodB9dT56M4Q1Q8xcPYyk7",
);

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

function deriveFactoryPda(programId: PublicKey): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
        [Buffer.from(FACTORY_SEED)],
        programId,
    );
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

function deriveLpMintPda(
    programId: PublicKey,
    pair: PublicKey,
): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
        [Buffer.from(LP_MINT_SEED_PREFIX), pair.toBuffer()],
        programId,
    );
}

function getAta(owner: PublicKey, mint: PublicKey): PublicKey {
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

function createCreatePairInstruction(args: {
    programId: PublicKey;
    payer: PublicKey;
    factory: PublicKey;
    pair: PublicKey;
    mintA: PublicKey;
    mintB: PublicKey;
    vault0: PublicKey;
    vault1: PublicKey;
    lpMint: PublicKey;
}): TransactionInstruction {
    return new TransactionInstruction({
        programId: args.programId,
        keys: [
            { pubkey: args.payer, isSigner: true, isWritable: true },
            { pubkey: args.factory, isSigner: false, isWritable: true },
            { pubkey: args.pair, isSigner: false, isWritable: true },
            { pubkey: args.mintA, isSigner: false, isWritable: false },
            { pubkey: args.mintB, isSigner: false, isWritable: false },
            { pubkey: args.vault0, isSigner: false, isWritable: true },
            { pubkey: args.vault1, isSigner: false, isWritable: true },
            { pubkey: args.lpMint, isSigner: false, isWritable: true },
            { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
            { pubkey: ATA_PROGRAM_ID, isSigner: false, isWritable: false },
            { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
            { pubkey: RENT_SYSVAR_ID, isSigner: false, isWritable: false },
        ],
        data: Buffer.from([1]),
    });
}

async function main(): Promise<void> {
    const rpcUrl = requireEnv("RPC_URL");
    const programId = new PublicKey(requireEnv("PROGRAM_ID"));
    const admin = getAdminKeypair();
    const mintA = new PublicKey(requireEnv("MINT_A"));
    const mintB = new PublicKey(requireEnv("MINT_B"));

    if (!admin.publicKey.equals(PROGRAM_ADMIN_AUTHORITY)) {
        throw new Error(
            `Admin keypair mismatch. Expected ${PROGRAM_ADMIN_AUTHORITY.toBase58()}, got ${admin.publicKey.toBase58()}`,
        );
    }

    const connection = new Connection(rpcUrl, "confirmed");
    const [factory] = deriveFactoryPda(programId);
    const [mint0, mint1] = canonicalMints(mintA, mintB);
    const [pair] = derivePairPda(programId, factory, mint0, mint1);
    const [lpMint] = deriveLpMintPda(programId, pair);
    const vault0 = getAta(pair, mint0);
    const vault1 = getAta(pair, mint1);

    console.log(`RPC URL: ${rpcUrl}`);
    console.log(`Program ID: ${programId.toBase58()}`);
    console.log(`Admin: ${admin.publicKey.toBase58()}`);
    console.log(`Factory PDA: ${factory.toBase58()}`);
    console.log(`Mint A: ${mintA.toBase58()}`);
    console.log(`Mint B: ${mintB.toBase58()}`);
    console.log(`Canonical mint0: ${mint0.toBase58()}`);
    console.log(`Canonical mint1: ${mint1.toBase58()}`);
    console.log(`Pair PDA: ${pair.toBase58()}`);
    console.log(`Vault0 ATA: ${vault0.toBase58()}`);
    console.log(`Vault1 ATA: ${vault1.toBase58()}`);
    console.log(`LP Mint PDA: ${lpMint.toBase58()}`);

    if ((await connection.getAccountInfo(factory, "confirmed")) === null) {
        throw new Error("Factory PDA does not exist yet. Run init-factory first.");
    }

    if ((await connection.getAccountInfo(pair, "confirmed")) !== null) {
        console.log("Pair already exists. Nothing to do.");
        return;
    }

    const ix = createCreatePairInstruction({
        programId,
        payer: admin.publicKey,
        factory,
        pair,
        mintA,
        mintB,
        vault0,
        vault1,
        lpMint,
    });

    const tx = new Transaction().add(ix);
    const signature = await sendAndConfirmTransaction(connection, tx, [admin]);

    console.log(`CreatePair signature: ${signature}`);
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
