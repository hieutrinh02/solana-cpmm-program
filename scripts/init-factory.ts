import "dotenv/config";

import { readFileSync } from "node:fs";

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

function createInitFactoryInstruction(
    programId: PublicKey,
    payer: PublicKey,
    factory: PublicKey,
): TransactionInstruction {
    return new TransactionInstruction({
        programId,
        keys: [
            { pubkey: payer, isSigner: true, isWritable: true },
            { pubkey: factory, isSigner: false, isWritable: true },
            { pubkey: SystemProgram.programId, isSigner: false, isWritable: false },
        ],
        data: Buffer.from([0]),
    });
}

async function main(): Promise<void> {
    const rpcUrl = requireEnv("RPC_URL");
    const programId = new PublicKey(requireEnv("PROGRAM_ID"));
    const admin = getAdminKeypair();

    if (!admin.publicKey.equals(PROGRAM_ADMIN_AUTHORITY)) {
        throw new Error(
            `Admin keypair mismatch. Expected ${PROGRAM_ADMIN_AUTHORITY.toBase58()}, got ${admin.publicKey.toBase58()}`,
        );
    }

    const connection = new Connection(rpcUrl, "confirmed");
    const [factory] = deriveFactoryPda(programId);

    console.log(`RPC URL: ${rpcUrl}`);
    console.log(`Program ID: ${programId.toBase58()}`);
    console.log(`Admin: ${admin.publicKey.toBase58()}`);
    console.log(`Factory PDA: ${factory.toBase58()}`);

    if ((await connection.getAccountInfo(factory, "confirmed")) !== null) {
        console.log("Factory already exists. Nothing to do.");
        return;
  }

  const ix = createInitFactoryInstruction(programId, admin.publicKey, factory);
  const tx = new Transaction().add(ix);
  const signature = await sendAndConfirmTransaction(connection, tx, [admin]);

  console.log(`InitFactory signature: ${signature}`);
}

main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
});
