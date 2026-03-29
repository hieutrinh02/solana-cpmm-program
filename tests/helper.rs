#![allow(dead_code, deprecated)]
use borsh::{to_vec, BorshDeserialize};
use litesvm::{types::TransactionResult, LiteSVM};
use litesvm_token::{
    get_spl_account,
    spl_token::state::{Account as TokenAccount, Mint},
    CreateAssociatedTokenAccount, CreateMint, MintTo, Transfer as TokenTransfer,
};
use solana_address::Address;
use solana_sdk::{
    account::Account,
    instruction::{AccountMeta, Instruction},
    message::Message,
    pubkey::Pubkey,
    signature::{read_keypair_file, Keypair, Signer},
    system_program, sysvar,
    transaction::Transaction,
};
use spl_associated_token_account_interface::program::id as spl_associated_token_account_program_id;
use spl_token_interface::{id as spl_token_program_id, instruction as spl_token_ix};

use solana_cpmm_program::instructions::lib::get_ata;
use solana_cpmm_program::{
    constants::{MINIMUM_LIQUIDITY, PROGRAM_ADMIN_AUTHORITY, SWAP_FEE_BPS},
    instructions::lib::{derive_factory_pda, derive_lp_mint_pda, derive_pair_pda},
    state::{FactoryState, PairState},
    Cmd,
};

pub const INITIAL_USER_FUNDS: u64 = 1_000_000_000;
pub const INITIAL_LIQUIDITY: u64 = 10_000_000;

/// Shared test fixture containing the canonical accounts used across scenarios.
#[derive(Debug)]
pub struct TestContext {
    pub program_id: Pubkey,
    pub admin: Pubkey,
    pub users: Vec<Keypair>,
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub mint0: Pubkey,
    pub mint1: Pubkey,
    pub user_token0: Vec<Pubkey>,
    pub user_token1: Vec<Pubkey>,
    pub factory: Pubkey,
    pub pair: Pubkey,
    pub lp_mint: Pubkey,
    pub vault0: Pubkey,
    pub vault1: Pubkey,
    pub locked_lp: Pubkey,
    pub admin_lp: Pubkey,
}

/// Builds a fresh LiteSVM instance, loads the compiled program artifact, and
/// pre-funds users with canonical token balances for happy-path tests.
pub fn setup() -> (LiteSVM, TestContext) {
    let mut svm = LiteSVM::new()
        .with_default_programs()
        .with_sigverify(false)
        // Admin transactions in this suite use default signatures on purpose.
        .with_transaction_history(0);

    let payer = Keypair::new();
    let users = vec![Keypair::new(), Keypair::new()];

    let program_id = read_keypair_file(program_keypair_path()).unwrap().pubkey();
    svm.add_program_from_file(program_id, program_so_path())
        .unwrap();

    svm.airdrop(&payer.pubkey(), 50_000_000_000).unwrap();
    svm.airdrop(&PROGRAM_ADMIN_AUTHORITY, 50_000_000_000)
        .unwrap();
    for user in users.iter() {
        svm.airdrop(&user.pubkey(), 10_000_000_000).unwrap();
    }

    let mint_a = create_mint(&mut svm, &payer, 6);
    let mint_b = create_mint(&mut svm, &payer, 6);
    // The program sorts mints canonically before deriving the pair PDA.
    let (mint0, mint1) = canonical_mints(mint_a, mint_b);

    let mut user_token0 = Vec::new();
    let mut user_token1 = Vec::new();
    for user in users.iter() {
        let ata0 = create_ata(&mut svm, &payer, &user.pubkey(), &mint0);
        let ata1 = create_ata(&mut svm, &payer, &user.pubkey(), &mint1);

        mint_to(&mut svm, &payer, &mint0, &ata0, INITIAL_USER_FUNDS);
        mint_to(&mut svm, &payer, &mint1, &ata1, INITIAL_USER_FUNDS);

        user_token0.push(ata0);
        user_token1.push(ata1);
    }

    // Precompute the addresses expected by the on-chain program.
    let (factory, _) = derive_factory_pda(&program_id);
    let (pair, _) = derive_pair_pda(&program_id, &factory, &mint0, &mint1);
    let (lp_mint, _) = derive_lp_mint_pda(&program_id, &pair);
    let vault0 = get_ata(&pair, &mint0);
    let vault1 = get_ata(&pair, &mint1);
    let locked_lp = get_ata(&pair, &lp_mint);
    let admin_lp = get_ata(&PROGRAM_ADMIN_AUTHORITY, &lp_mint);

    (
        svm,
        TestContext {
            program_id,
            admin: PROGRAM_ADMIN_AUTHORITY,
            users,
            mint_a,
            mint_b,
            mint0,
            mint1,
            user_token0,
            user_token1,
            factory,
            pair,
            lp_mint,
            vault0,
            vault1,
            locked_lp,
            admin_lp,
        },
    )
}

/// Creates the singleton factory and then the pair under test.
pub fn bootstrap_pair(svm: &mut LiteSVM, ctx: &TestContext) {
    send_admin_transaction(svm, ctx.admin, &[create_init_factory_ix(ctx)]).unwrap();
    send_admin_transaction(svm, ctx.admin, &[create_create_pair_ix(ctx)]).unwrap();
}

/// Creates just the singleton factory account without creating a pair.
pub fn bootstrap_factory(svm: &mut LiteSVM, ctx: &TestContext) {
    send_admin_transaction(svm, ctx.admin, &[create_init_factory_ix(ctx)]).unwrap();
}

/// Creates a test mint owned by the classic SPL Token program.
pub fn create_mint(svm: &mut LiteSVM, payer: &Keypair, decimals: u8) -> Pubkey {
    CreateMint::new(svm, payer)
        .authority(&payer.pubkey())
        .decimals(decimals)
        .token_program_id(&token_program_id())
        .send()
        .unwrap()
}

/// Creates the ATA expected by the program for `(owner, mint)`.
pub fn create_ata(svm: &mut LiteSVM, payer: &Keypair, owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    CreateAssociatedTokenAccount::new(svm, payer, mint)
        .owner(owner)
        .token_program_id(&token_program_id())
        .send()
        .unwrap()
}

/// Mints test inventory into a pre-created token account.
pub fn mint_to(svm: &mut LiteSVM, payer: &Keypair, mint: &Pubkey, dst: &Pubkey, amount: u64) {
    MintTo::new(svm, payer, mint, dst, amount)
        .owner(payer)
        .token_program_id(&token_program_id())
        .send()
        .unwrap();
}

/// Sends a plain SPL transfer used to create live balance deltas.
pub fn transfer_tokens(
    svm: &mut LiteSVM,
    payer: &Keypair,
    src: &Pubkey,
    dst: &Pubkey,
    authority: &Keypair,
    amount: u64,
) {
    let mint = get_mint_for_token_account(svm, src);
    let token_program = token_program_id();
    let mut builder = TokenTransfer::new(svm, payer, &mint, dst, amount)
        .source(src)
        .token_program_id(&token_program);
    builder = if payer.pubkey() == authority.pubkey() {
        builder.owner(payer)
    } else {
        builder.owner(authority)
    };
    builder.send().unwrap();
}

/// Reads the token balance from a packed SPL account.
pub fn get_token_balance(svm: &LiteSVM, account: &Pubkey) -> u64 {
    let token_account: TokenAccount = get_spl_account(svm, account).unwrap();
    token_account.amount
}

/// Reads the total supply from an SPL mint account.
pub fn get_mint_supply(svm: &LiteSVM, mint: &Pubkey) -> u64 {
    let mint_state: Mint = get_spl_account(svm, mint).unwrap();
    mint_state.supply
}

/// Decodes the factory account written by `InitFactory`.
pub fn get_factory_state(svm: &LiteSVM, factory: &Pubkey) -> FactoryState {
    let data = svm.get_account(factory).unwrap().data;
    FactoryState::try_from_slice(&data).unwrap()
}

/// Decodes the pair account written by `CreatePair` and later instructions.
pub fn get_pair_state(svm: &LiteSVM, pair: &Pubkey) -> PairState {
    let data = svm.get_account(pair).unwrap().data;
    PairState::try_from_slice(&data).unwrap()
}

/// Builds the admin-only `InitFactory` instruction.
pub fn create_init_factory_ix(ctx: &TestContext) -> Instruction {
    Instruction::new_with_borsh(
        ctx.program_id,
        &Cmd::InitFactory,
        vec![
            AccountMeta::new(ctx.admin, true),
            AccountMeta::new(ctx.factory, false),
            AccountMeta::new_readonly(system_program::id(), false),
        ],
    )
}

/// Builds the admin-only `CreatePair` instruction with the canonical vault and
/// LP mint addresses expected by the program.
pub fn create_create_pair_ix(ctx: &TestContext) -> Instruction {
    Instruction::new_with_borsh(
        ctx.program_id,
        &Cmd::CreatePair,
        vec![
            AccountMeta::new(ctx.admin, true),
            AccountMeta::new(ctx.factory, false),
            AccountMeta::new(ctx.pair, false),
            AccountMeta::new_readonly(ctx.mint_a, false),
            AccountMeta::new_readonly(ctx.mint_b, false),
            AccountMeta::new(ctx.vault0, false),
            AccountMeta::new(ctx.vault1, false),
            AccountMeta::new(ctx.lp_mint, false),
            AccountMeta::new_readonly(token_program_id(), false),
            AccountMeta::new_readonly(ata_program_id(), false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(sysvar::rent::id(), false),
        ],
    )
}

/// Builds `AddLiquidity` for one of the seeded test users.
pub fn create_add_liquidity_ix(
    ctx: &TestContext,
    user_index: usize,
    amount0_desired: u64,
    amount1_desired: u64,
    amount0_min: u64,
    amount1_min: u64,
) -> Instruction {
    Instruction::new_with_borsh(
        ctx.program_id,
        &Cmd::AddLiquidity {
            amount0_desired,
            amount1_desired,
            amount0_min,
            amount1_min,
        },
        vec![
            AccountMeta::new(ctx.users[user_index].pubkey(), true),
            AccountMeta::new(ctx.pair, false),
            AccountMeta::new(ctx.user_token0[user_index], false),
            AccountMeta::new(ctx.user_token1[user_index], false),
            AccountMeta::new(ctx.vault0, false),
            AccountMeta::new(ctx.vault1, false),
            AccountMeta::new(ctx.lp_mint, false),
            AccountMeta::new(
                get_ata(&ctx.users[user_index].pubkey(), &ctx.lp_mint),
                false,
            ),
            AccountMeta::new(ctx.locked_lp, false),
            AccountMeta::new_readonly(ctx.admin, false),
            AccountMeta::new(ctx.admin_lp, false),
            AccountMeta::new_readonly(token_program_id(), false),
            AccountMeta::new_readonly(ata_program_id(), false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(sysvar::rent::id(), false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
        ],
    )
}

/// Builds `Swap` after the caller has already moved `amount_in` into the
/// appropriate vault.
pub fn create_swap_ix(
    ctx: &TestContext,
    user_index: usize,
    amount0_out: u64,
    amount1_out: u64,
) -> Instruction {
    Instruction::new_with_borsh(
        ctx.program_id,
        &Cmd::Swap {
            amount0_out,
            amount1_out,
        },
        vec![
            AccountMeta::new(ctx.users[user_index].pubkey(), true),
            AccountMeta::new(ctx.pair, false),
            AccountMeta::new(ctx.user_token0[user_index], false),
            AccountMeta::new(ctx.user_token1[user_index], false),
            AccountMeta::new(ctx.vault0, false),
            AccountMeta::new(ctx.vault1, false),
            AccountMeta::new_readonly(token_program_id(), false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
        ],
    )
}

/// Builds `RemoveLiquidity` for one of the seeded test users.
pub fn create_remove_liquidity_ix(
    ctx: &TestContext,
    user_index: usize,
    liquidity: u64,
    amount0_min: u64,
    amount1_min: u64,
) -> Instruction {
    Instruction::new_with_borsh(
        ctx.program_id,
        &Cmd::RemoveLiquidity {
            liquidity,
            amount0_min,
            amount1_min,
        },
        vec![
            AccountMeta::new(ctx.users[user_index].pubkey(), true),
            AccountMeta::new(ctx.pair, false),
            AccountMeta::new_readonly(ctx.mint0, false),
            AccountMeta::new_readonly(ctx.mint1, false),
            AccountMeta::new(ctx.user_token0[user_index], false),
            AccountMeta::new(ctx.user_token1[user_index], false),
            AccountMeta::new(ctx.vault0, false),
            AccountMeta::new(ctx.vault1, false),
            AccountMeta::new(ctx.lp_mint, false),
            AccountMeta::new(
                get_ata(&ctx.users[user_index].pubkey(), &ctx.lp_mint),
                false,
            ),
            AccountMeta::new_readonly(ctx.admin, false),
            AccountMeta::new(ctx.admin_lp, false),
            AccountMeta::new_readonly(token_program_id(), false),
            AccountMeta::new_readonly(ata_program_id(), false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(sysvar::rent::id(), false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
        ],
    )
}

/// Builds `Sync` to refresh reserves from live vault balances.
pub fn create_sync_ix(ctx: &TestContext) -> Instruction {
    Instruction::new_with_borsh(
        ctx.program_id,
        &Cmd::Sync,
        vec![
            AccountMeta::new(ctx.pair, false),
            AccountMeta::new_readonly(ctx.vault0, false),
            AccountMeta::new_readonly(ctx.vault1, false),
            AccountMeta::new_readonly(sysvar::clock::id(), false),
        ],
    )
}

/// Builds `Skim` to return vault excess to a recipient without changing stored
/// reserves.
pub fn create_skim_ix(ctx: &TestContext, recipient: &Pubkey) -> Instruction {
    Instruction::new_with_borsh(
        ctx.program_id,
        &Cmd::Skim,
        vec![
            AccountMeta::new(ctx.users[1].pubkey(), true),
            AccountMeta::new_readonly(*recipient, false),
            AccountMeta::new_readonly(ctx.pair, false),
            AccountMeta::new_readonly(ctx.mint0, false),
            AccountMeta::new_readonly(ctx.mint1, false),
            AccountMeta::new(get_ata(recipient, &ctx.mint0), false),
            AccountMeta::new(get_ata(recipient, &ctx.mint1), false),
            AccountMeta::new(ctx.vault0, false),
            AccountMeta::new(ctx.vault1, false),
            AccountMeta::new_readonly(token_program_id(), false),
            AccountMeta::new_readonly(ata_program_id(), false),
            AccountMeta::new_readonly(system_program::id(), false),
            AccountMeta::new_readonly(sysvar::rent::id(), false),
        ],
    )
}

/// Mirrors the program's swap output formula for deterministic assertions.
pub fn compute_swap_amount_out(amount_in: u64, reserve_in: u64, reserve_out: u64) -> u64 {
    let fee_denominator = 10_000u128;
    let amount_in_with_fee = (amount_in as u128) * (fee_denominator - SWAP_FEE_BPS as u128);
    let numerator = amount_in_with_fee * reserve_out as u128;
    let denominator = (reserve_in as u128 * fee_denominator) + amount_in_with_fee;
    (numerator / denominator) as u64
}

/// The first LP provider permanently loses `MINIMUM_LIQUIDITY` to the locked
/// pair-owned LP account.
pub fn expected_initial_user_lp() -> u64 {
    INITIAL_LIQUIDITY - MINIMUM_LIQUIDITY
}

/// Sends a standard signed transaction for normal user flows.
pub fn send_signed_transaction(
    svm: &mut LiteSVM,
    payer: &Keypair,
    instructions: &[Instruction],
    signers: &[&Keypair],
) -> TransactionResult {
    svm.send_transaction(Transaction::new_signed_with_payer(
        instructions,
        Some(&payer.pubkey()),
        signers,
        svm.latest_blockhash(),
    ))
}

/// Sends an unsigned transaction with an explicit fee payer. Useful for tests
/// that need account metas present without signer privileges.
pub fn send_unsigned_transaction(
    svm: &mut LiteSVM,
    payer: Pubkey,
    instructions: &[Instruction],
) -> TransactionResult {
    let message = Message::new_with_blockhash(instructions, Some(&payer), &svm.latest_blockhash());
    svm.send_transaction(Transaction::new_unsigned(message))
}

/// Sends an admin transaction without a real signature. This works because the
/// test VM disables signature verification and only the signer flag matters.
pub fn send_admin_transaction(
    svm: &mut LiteSVM,
    admin: Pubkey,
    instructions: &[Instruction],
) -> TransactionResult {
    let message = Message::new_with_blockhash(instructions, Some(&admin), &svm.latest_blockhash());
    svm.send_transaction(Transaction::new_unsigned(message))
}

/// Rewrites the signer pubkey in an instruction.
pub fn set_instruction_signer(ix: &mut Instruction, index: usize, signer: Pubkey) {
    ix.accounts[index].pubkey = signer;
}

/// Writes an arbitrary account into LiteSVM for negative-path fixture setup.
pub fn seed_account(svm: &mut LiteSVM, pubkey: Pubkey, account: Account) {
    svm.set_account(pubkey, account).unwrap();
}

/// Seeds a system-owned zero-data account to model a prefunded blank address.
pub fn seed_prefunded_system_account(svm: &mut LiteSVM, pubkey: Pubkey, lamports: u64) {
    seed_account(
        svm,
        pubkey,
        Account {
            lamports,
            owner: system_program::id(),
            ..Account::default()
        },
    );
}

/// Writes a factory account fixture with custom owner/state combinations.
pub fn seed_factory_account(
    svm: &mut LiteSVM,
    ctx: &TestContext,
    owner: Pubkey,
    state: FactoryState,
) {
    seed_account(
        svm,
        ctx.factory,
        Account {
            lamports: 1,
            data: to_vec(&state).unwrap(),
            owner,
            ..Account::default()
        },
    );
}

fn canonical_mints(mint_a: Pubkey, mint_b: Pubkey) -> (Pubkey, Pubkey) {
    if mint_a.to_bytes() < mint_b.to_bytes() {
        (mint_a, mint_b)
    } else {
        (mint_b, mint_a)
    }
}

fn program_keypair_path() -> String {
    format!(
        "{}/target/deploy/solana_cpmm_program-keypair.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn program_so_path() -> String {
    format!(
        "{}/target/deploy/solana_cpmm_program.so",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Builds a raw SPL transfer instruction when a test needs exact control over
/// instruction ordering within one transaction.
pub fn token_transfer_ix(
    src: &Pubkey,
    dst: &Pubkey,
    authority: &Pubkey,
    amount: u64,
) -> Instruction {
    let ix = spl_token_ix::transfer(
        &token_program_address(),
        &address_from_pubkey(src),
        &address_from_pubkey(dst),
        &address_from_pubkey(authority),
        &[],
        amount,
    )
    .unwrap();
    Instruction {
        program_id: pubkey_from_address(&ix.program_id),
        accounts: ix
            .accounts
            .iter()
            .map(|acc| AccountMeta {
                pubkey: pubkey_from_address(&acc.pubkey),
                is_signer: acc.is_signer,
                is_writable: acc.is_writable,
            })
            .collect(),
        data: ix.data,
    }
}

fn token_program_address() -> Address {
    spl_token_program_id()
}

fn token_program_id() -> Pubkey {
    pubkey_from_address(&token_program_address())
}

fn ata_program_id() -> Pubkey {
    pubkey_from_address(&spl_associated_token_account_program_id())
}

fn address_from_pubkey(pubkey: &Pubkey) -> Address {
    Address::from(pubkey.to_bytes())
}

fn pubkey_from_address(address: &Address) -> Pubkey {
    Pubkey::from(address.to_bytes())
}

fn get_mint_for_token_account(svm: &LiteSVM, account: &Pubkey) -> Pubkey {
    let token_account: TokenAccount = get_spl_account(svm, account).unwrap();
    token_account.mint
}
