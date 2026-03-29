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
const TOKEN_PROGRAM_ID = new PublicKey(
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
);
const ATA_PROGRAM_ID = new PublicKey(
    "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL",
);
const RENT_SYSVAR_ID = new PublicKey(
    "SysvarRent111111111111111111111111111111111",
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

function readU64LE(data: Buffer, offset: number): bigint {
    return data.readBigUInt64LE(offset);
}

function createSkimInstruction(args: {
    programId: PublicKey;
    payer: PublicKey;
    recipient: PublicKey;
    pair: PublicKey;
    mint0: PublicKey;
    mint1: PublicKey;
    recipientToken0: PublicKey;
    recipientToken1: PublicKey;
    vault0: PublicKey;
    vault1: PublicKey;
}): TransactionInstruction {
    return new TransactionInstruction({
        programId: args.programId,
        keys: [
            { pubkey: args.payer, isSigner: true, isWritable: true },
            { pubkey: args.recipient, isSigner: false, isWritable: false },
            { pubkey: args.pair, isSigner: false, isWritable: false },
            { pubkey: args.mint0, isSigner: false, isWritable: false },
            { pubkey: args.mint1, isSigner: false, isWritable: false },
            { pubkey: args.recipientToken0, isSigner: false, isWritable: true },
            { pubkey: args.recipientToken1, isSigner: false, isWritable: true },
            { pubkey: args.vault0, isSigner: false, isWritable: true },
            { pubkey: args.vault1, isSigner: false, isWritable: true },
            { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
            { pubkey: ATA_PROGRAM_ID, isSigner: false, isWritable: false },
            { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
            { pubkey: RENT_SYSVAR_ID, isSigner: false, isWritable: false },
        ],
        data: Buffer.from([5]),
    });
}

async function main(): Promise<void> {
    const rpcUrl = requireEnv("RPC_URL");
    const programId = new PublicKey(requireEnv("PROGRAM_ID"));
    const payer = getKeypair("USER_KEYPAIR_PATH");
    const mintA = new PublicKey(requireEnv("MINT_A"));
    const mintB = new PublicKey(requireEnv("MINT_B"));
    const recipient = payer.publicKey;

    const connection = new Connection(rpcUrl, "confirmed");
    const [factory] = deriveFactoryPda(programId);
    const [mint0, mint1] = canonicalMints(mintA, mintB);
    const [pair] = derivePairPda(programId, factory, mint0, mint1);
    const recipientToken0 = getWalletAta(recipient, mint0);
    const recipientToken1 = getWalletAta(recipient, mint1);
    const vault0 = getPdaAta(pair, mint0);
    const vault1 = getPdaAta(pair, mint1);
    const pairAccount = await connection.getAccountInfo(pair, "confirmed");

    if (!pairAccount) {
        throw new Error("Pair PDA does not exist.");
    }

    const reserve0 = readU64LE(pairAccount.data, 193);
    const reserve1 = readU64LE(pairAccount.data, 201);
    const vault0Balance = BigInt((await connection.getTokenAccountBalance(vault0, "confirmed")).value.amount);
    const vault1Balance = BigInt((await connection.getTokenAccountBalance(vault1, "confirmed")).value.amount);
    const excess0 = vault0Balance > reserve0 ? vault0Balance - reserve0 : 0n;
    const excess1 = vault1Balance > reserve1 ? vault1Balance - reserve1 : 0n;

    const ix = createSkimInstruction({
        programId,
        payer: payer.publicKey,
        recipient,
        pair,
        mint0,
        mint1,
        recipientToken0,
        recipientToken1,
        vault0,
        vault1,
    });

    const tx = new Transaction().add(ix);
    const signature = await sendAndConfirmTransaction(connection, tx, [payer]);

    console.log(`RPC URL: ${rpcUrl}`);
    console.log(`Program ID: ${programId.toBase58()}`);
    console.log(`Payer: ${payer.publicKey.toBase58()}`);
    console.log(`Recipient: ${recipient.toBase58()}`);
    console.log(`Pair PDA: ${pair.toBase58()}`);
    console.log(`Reserve0: ${reserve0.toString()}`);
    console.log(`Reserve1: ${reserve1.toString()}`);
    console.log(`Vault0 balance: ${vault0Balance.toString()}`);
    console.log(`Vault1 balance: ${vault1Balance.toString()}`);
    console.log(`Excess0: ${excess0.toString()}`);
    console.log(`Excess1: ${excess1.toString()}`);
    console.log(`Recipient token0 ATA: ${recipientToken0.toBase58()}`);
    console.log(`Recipient token1 ATA: ${recipientToken1.toBase58()}`);
    console.log(`Skim signature: ${signature}`);
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
