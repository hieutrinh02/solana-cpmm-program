import "dotenv/config";

import { readFileSync } from "node:fs";

import {
    createTransferInstruction,
    getAssociatedTokenAddressSync,
} from "@solana/spl-token";
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
const SWAP_FEE_BPS = 3n;
const FEE_DENOMINATOR = 10_000n;
const TOKEN_PROGRAM_ID = new PublicKey(
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const ATA_PROGRAM_ID = new PublicKey(
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);
const CLOCK_SYSVAR_ID = new PublicKey(
    "SysvarC1ock11111111111111111111111111111111",
);
const SWAP_DIRECTION = "0to1" as const;
const MINT_DECIMALS = 6;
const AMOUNT_IN = BigInt(100 * (10 ** MINT_DECIMALS));

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

function readU64LE(data: Buffer, offset: number): bigint {
    return data.readBigUInt64LE(offset);
}

function computeAmountOut(
    amountIn: bigint,
    reserveIn: bigint,
    reserveOut: bigint,
): bigint {
    const amountInWithFee = amountIn * (FEE_DENOMINATOR - SWAP_FEE_BPS);
    const numerator = amountInWithFee * reserveOut;
    const denominator = reserveIn * FEE_DENOMINATOR + amountInWithFee;
    return numerator / denominator;
}

function createSwapInstruction(args: {
    programId: PublicKey;
    user: PublicKey;
    pair: PublicKey;
    userToken0: PublicKey;
    userToken1: PublicKey;
    vault0: PublicKey;
    vault1: PublicKey;
    amount0Out: bigint;
    amount1Out: bigint;
}): TransactionInstruction {
    const data = Buffer.concat([
        Buffer.from([3]),
        encodeU64(args.amount0Out),
        encodeU64(args.amount1Out),
    ]);

    return new TransactionInstruction({
        programId: args.programId,
        keys: [
            { pubkey: args.user, isSigner: true, isWritable: true },
            { pubkey: args.pair, isSigner: false, isWritable: true },
            { pubkey: args.userToken0, isSigner: false, isWritable: true },
            { pubkey: args.userToken1, isSigner: false, isWritable: true },
            { pubkey: args.vault0, isSigner: false, isWritable: true },
            { pubkey: args.vault1, isSigner: false, isWritable: true },
            { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
            { pubkey: CLOCK_SYSVAR_ID, isSigner: false, isWritable: false },
        ],
        data,
    });
}

async function main(): Promise<void> {
    const rpcUrl = requireEnv("RPC_URL");
    const programId = new PublicKey(requireEnv("PROGRAM_ID"));
    const user = getKeypair("USER_KEYPAIR_PATH");
    const mintA = new PublicKey(requireEnv("MINT_A"));
    const mintB = new PublicKey(requireEnv("MINT_B"));
    const direction = SWAP_DIRECTION;
    const amountIn = AMOUNT_IN;

    const connection = new Connection(rpcUrl, "confirmed");
    const [factory] = deriveFactoryPda(programId);
    const [mint0, mint1] = canonicalMints(mintA, mintB);
    const [pair] = derivePairPda(programId, factory, mint0, mint1);
    const userToken0 = getWalletAta(user.publicKey, mint0);
    const userToken1 = getWalletAta(user.publicKey, mint1);
    const vault0 = getPdaAta(pair, mint0);
    const vault1 = getPdaAta(pair, mint1);
    const pairAccount = await connection.getAccountInfo(pair, "confirmed");

    if (!pairAccount) {
        throw new Error("Pair PDA does not exist.");
    }

    const reserve0 = readU64LE(pairAccount.data, 193);
    const reserve1 = readU64LE(pairAccount.data, 201);

    if (reserve0 === 0n || reserve1 === 0n) {
        throw new Error("Pair has zero reserves.");
    }

    const amountOut =
        direction === "0to1"
            ? computeAmountOut(amountIn, reserve0, reserve1)
            : computeAmountOut(amountIn, reserve1, reserve0);

    const [inputSource, inputVault, amount0Out, amount1Out] =
        direction === "0to1"
            ? [userToken0, vault0, 0n, amountOut]
            : [userToken1, vault1, amountOut, 0n];

    const transferIx = createTransferInstruction(
        inputSource,
        inputVault,
        user.publicKey,
        amountIn,
    );

    const swapIx = createSwapInstruction({
        programId,
        user: user.publicKey,
        pair,
        userToken0,
        userToken1,
        vault0,
        vault1,
        amount0Out,
        amount1Out,
    });

    const tx = new Transaction().add(transferIx, swapIx);
    const signature = await sendAndConfirmTransaction(connection, tx, [user]);

    console.log(`RPC URL: ${rpcUrl}`);
    console.log(`Program ID: ${programId.toBase58()}`);
    console.log(`User: ${user.publicKey.toBase58()}`);
    console.log(`Pair PDA: ${pair.toBase58()}`);
    console.log(`Direction: ${direction}`);
    console.log(`Amount in: ${amountIn.toString()}`);
    console.log(`Reserve0: ${reserve0.toString()}`);
    console.log(`Reserve1: ${reserve1.toString()}`);
    console.log(`Amount out: ${amountOut.toString()}`);
    console.log(`User token0 ATA: ${userToken0.toBase58()}`);
    console.log(`User token1 ATA: ${userToken1.toBase58()}`);
    console.log(`Vault0 ATA: ${vault0.toBase58()}`);
    console.log(`Vault1 ATA: ${vault1.toBase58()}`);
    console.log(`Swap signature: ${signature}`);
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
