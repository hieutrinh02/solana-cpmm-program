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
const CLOCK_SYSVAR_ID = new PublicKey(
    "SysvarC1ock11111111111111111111111111111111",
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

function deriveLpMintPda(programId: PublicKey, pair: PublicKey): [PublicKey, number] {
    return PublicKey.findProgramAddressSync(
        [Buffer.from(LP_MINT_SEED_PREFIX), pair.toBuffer()],
        programId,
    );
}

function getWalletAta(owner: PublicKey, mint: PublicKey): PublicKey {
    return getAssociatedTokenAddressSync(
        mint,
        owner,
        false,
        TOKEN_PROGRAM_ID,
        ATA_PROGRAM_ID,
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

function encodeU64(value: bigint): Buffer {
    const buffer = Buffer.alloc(8);
    buffer.writeBigUInt64LE(value);
    return buffer;
}

function createAddLiquidityInstruction(args: {
    programId: PublicKey;
    payer: PublicKey;
    pair: PublicKey;
    payerToken0: PublicKey;
    payerToken1: PublicKey;
    vault0: PublicKey;
    vault1: PublicKey;
    lpMint: PublicKey;
    payerLp: PublicKey;
    lockedLp: PublicKey;
    admin: PublicKey;
    adminLp: PublicKey;
    amount0Desired: bigint;
    amount1Desired: bigint;
    amount0Min: bigint;
    amount1Min: bigint;
}): TransactionInstruction {
    const data = Buffer.concat([
        Buffer.from([2]),
        encodeU64(args.amount0Desired),
        encodeU64(args.amount1Desired),
        encodeU64(args.amount0Min),
        encodeU64(args.amount1Min),
    ]);

    return new TransactionInstruction({
        programId: args.programId,
        keys: [
            { pubkey: args.payer, isSigner: true, isWritable: true },
            { pubkey: args.pair, isSigner: false, isWritable: true },
            { pubkey: args.payerToken0, isSigner: false, isWritable: true },
            { pubkey: args.payerToken1, isSigner: false, isWritable: true },
            { pubkey: args.vault0, isSigner: false, isWritable: true },
            { pubkey: args.vault1, isSigner: false, isWritable: true },
            { pubkey: args.lpMint, isSigner: false, isWritable: true },
            { pubkey: args.payerLp, isSigner: false, isWritable: true },
            { pubkey: args.lockedLp, isSigner: false, isWritable: true },
            { pubkey: args.admin, isSigner: false, isWritable: false },
            { pubkey: args.adminLp, isSigner: false, isWritable: true },
            { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
            { pubkey: ATA_PROGRAM_ID, isSigner: false, isWritable: false },
            { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
            { pubkey: RENT_SYSVAR_ID, isSigner: false, isWritable: false },
            { pubkey: CLOCK_SYSVAR_ID, isSigner: false, isWritable: false },
        ],
        data,
    });
}

async function main(): Promise<void> {
    const rpcUrl = requireEnv("RPC_URL");
    const programId = new PublicKey(requireEnv("PROGRAM_ID"));
    const admin = getKeypair("ADMIN_KEYPAIR_PATH");
    const user = getKeypair("USER_KEYPAIR_PATH");
    const mintA = new PublicKey(requireEnv("MINT_A"));
    const mintB = new PublicKey(requireEnv("MINT_B"));
    const mintDecimals = 6;
    const amount0Desired = BigInt(1000 * (10 ** mintDecimals));
    const amount1Desired = BigInt(1000 * (10 ** mintDecimals));
    const amount0Min = BigInt(1000 * (10 ** mintDecimals));
    const amount1Min = BigInt(1000 * (10 ** mintDecimals));

    const connection = new Connection(rpcUrl, "confirmed");
    const [factory] = deriveFactoryPda(programId);
    const [mint0, mint1] = canonicalMints(mintA, mintB);
    const [pair] = derivePairPda(programId, factory, mint0, mint1);
    const [lpMint] = deriveLpMintPda(programId, pair);
    const vault0 = getPdaAta(pair, mint0);
    const vault1 = getPdaAta(pair, mint1);
    const payerToken0 = getWalletAta(user.publicKey, mint0);
    const payerToken1 = getWalletAta(user.publicKey, mint1);
    const payerLp = getWalletAta(user.publicKey, lpMint);
    const lockedLp = getPdaAta(pair, lpMint);
    const adminLp = getWalletAta(PROGRAM_ADMIN_AUTHORITY, lpMint);

    const ix = createAddLiquidityInstruction({
        programId,
        payer: user.publicKey,
        pair,
        payerToken0,
        payerToken1,
        vault0,
        vault1,
        lpMint,
        payerLp,
        lockedLp,
        admin: PROGRAM_ADMIN_AUTHORITY,
        adminLp,
        amount0Desired,
        amount1Desired,
        amount0Min,
        amount1Min,
    });

    const tx = new Transaction().add(ix);
    const signature = await sendAndConfirmTransaction(connection, tx, [user]);

    console.log(`RPC URL: ${rpcUrl}`);
    console.log(`Program ID: ${programId.toBase58()}`);
    console.log(`Admin: ${admin.publicKey.toBase58()}`);
    console.log(`User: ${user.publicKey.toBase58()}`);
    console.log(`Pair PDA: ${pair.toBase58()}`);
    console.log(`LP Mint PDA: ${lpMint.toBase58()}`);
    console.log(`Payer token0 ATA: ${payerToken0.toBase58()}`);
    console.log(`Payer token1 ATA: ${payerToken1.toBase58()}`);
    console.log(`Payer LP ATA: ${payerLp.toBase58()}`);
    console.log(`Amount0 desired: ${amount0Desired.toString()}`);
    console.log(`Amount1 desired: ${amount1Desired.toString()}`);
    console.log(`Amount0 min: ${amount0Min.toString()}`);
    console.log(`Amount1 min: ${amount1Min.toString()}`);
    console.log(`AddLiquidity signature: ${signature}`);
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
