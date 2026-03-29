use borsh::to_vec;
use solana_sdk::{
    account::Account,
    instruction::InstructionError,
    pubkey::Pubkey,
    signature::{Keypair, Signer},
    transaction::TransactionError,
};

mod helper;

use helper::{
    bootstrap_pair, create_add_liquidity_ix, create_ata, create_skim_ix, get_pair_state,
    get_token_balance, seed_account, seed_prefunded_system_account, send_signed_transaction,
    send_unsigned_transaction, setup, transfer_tokens, INITIAL_LIQUIDITY,
};

#[test]
fn test_skim() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let extra0 = 50_000;
    let extra1 = 25_000;
    let recipient_before0 = get_token_balance(&svm, &ctx.user_token0[0]);
    let recipient_before1 = get_token_balance(&svm, &ctx.user_token1[0]);

    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token0[1],
        &ctx.vault0,
        &ctx.users[1],
        extra0,
    );
    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token1[1],
        &ctx.vault1,
        &ctx.users[1],
        extra1,
    );

    // Interact
    let skim_ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    send_signed_transaction(&mut svm, &ctx.users[1], &[skim_ix], &[&ctx.users[1]]).unwrap();

    // Assert
    let pair_state = get_pair_state(&svm, &ctx.pair);
    assert_eq!(
        get_token_balance(&svm, &ctx.user_token0[0]),
        recipient_before0 + extra0
    );
    assert_eq!(
        get_token_balance(&svm, &ctx.user_token1[0]),
        recipient_before1 + extra1
    );
    assert_eq!(get_token_balance(&svm, &ctx.vault0), INITIAL_LIQUIDITY);
    assert_eq!(get_token_balance(&svm, &ctx.vault1), INITIAL_LIQUIDITY);
    assert_eq!(pair_state.reserve0, INITIAL_LIQUIDITY);
    assert_eq!(pair_state.reserve1, INITIAL_LIQUIDITY);
    assert_eq!(
        pair_state.k_last,
        (INITIAL_LIQUIDITY as u128) * (INITIAL_LIQUIDITY as u128)
    );
}

#[test]
fn test_skim_missing_payer_signature_succeeds_when_recipient_atas_already_exist() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let recipient = Keypair::new();
    svm.airdrop(&recipient.pubkey(), 1_000_000_000).unwrap();
    let recipient_token0 = create_ata(&mut svm, &ctx.users[1], &recipient.pubkey(), &ctx.mint0);
    let recipient_token1 = create_ata(&mut svm, &ctx.users[1], &recipient.pubkey(), &ctx.mint1);

    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token0[1],
        &ctx.vault0,
        &ctx.users[1],
        50_000,
    );
    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token1[1],
        &ctx.vault1,
        &ctx.users[1],
        25_000,
    );

    // Interact
    let mut ix = create_skim_ix(&ctx, &recipient.pubkey());
    ix.accounts[0].is_signer = false;
    send_unsigned_transaction(&mut svm, ctx.users[1].pubkey(), &[ix]).unwrap();

    // Assert
    assert_eq!(get_token_balance(&svm, &recipient_token0), 50_000);
    assert_eq!(get_token_balance(&svm, &recipient_token1), 25_000);
}

#[test]
fn test_skim_prefunded_recipient_ata_succeeds_with_payer_signature() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let recipient = Keypair::new();
    svm.airdrop(&recipient.pubkey(), 1_000_000_000).unwrap();
    let recipient_token0 =
        solana_cpmm_program::instructions::lib::get_ata(&recipient.pubkey(), &ctx.mint0);
    let recipient_token1 =
        solana_cpmm_program::instructions::lib::get_ata(&recipient.pubkey(), &ctx.mint1);
    seed_prefunded_system_account(&mut svm, recipient_token0, 1);

    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token0[1],
        &ctx.vault0,
        &ctx.users[1],
        50_000,
    );
    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token1[1],
        &ctx.vault1,
        &ctx.users[1],
        25_000,
    );

    // Interact
    let skim_ix = create_skim_ix(&ctx, &recipient.pubkey());
    send_signed_transaction(&mut svm, &ctx.users[1], &[skim_ix], &[&ctx.users[1]]).unwrap();

    // Assert
    assert_eq!(get_token_balance(&svm, &recipient_token0), 50_000);
    assert_eq!(get_token_balance(&svm, &recipient_token1), 25_000);
}

#[test]
fn test_skim_invalid_system_program_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[11].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_skim_invalid_token_program_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[9].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_skim_invalid_ata_program_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[10].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_skim_invalid_rent_sysvar_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[12].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_skim_pair_owner_mismatch_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let mut pair_account = svm.get_account(&ctx.pair).unwrap();
    pair_account.owner = Pubkey::new_unique();
    seed_account(&mut svm, ctx.pair, pair_account);

    // Interact
    let ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_skim_pair_not_initialized_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let mut pair_state = get_pair_state(&svm, &ctx.pair);
    pair_state.is_initialized = false;
    seed_account(
        &mut svm,
        ctx.pair,
        Account {
            lamports: 1,
            data: to_vec(&pair_state).unwrap(),
            owner: ctx.program_id,
            ..Account::default()
        },
    );

    // Interact
    let ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::UninitializedAccount)
    );
}

#[test]
fn test_skim_pair_factory_mismatch_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let mut pair_state = get_pair_state(&svm, &ctx.pair);
    pair_state.factory = Pubkey::new_unique();
    seed_account(
        &mut svm,
        ctx.pair,
        Account {
            lamports: 1,
            data: to_vec(&pair_state).unwrap(),
            owner: ctx.program_id,
            ..Account::default()
        },
    );

    // Interact
    let ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_skim_invalid_pair_pda_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let fake_pair = Pubkey::new_unique();
    let pair_state = get_pair_state(&svm, &ctx.pair);
    seed_account(
        &mut svm,
        fake_pair,
        Account {
            lamports: 1,
            data: to_vec(&pair_state).unwrap(),
            owner: ctx.program_id,
            ..Account::default()
        },
    );

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[2].pubkey = fake_pair;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidSeeds)
    );
}

#[test]
fn test_skim_invalid_vault0_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[7].pubkey = ctx.vault1;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_skim_invalid_vault1_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[8].pubkey = ctx.vault0;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_skim_invalid_token0_mint_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[3].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_skim_invalid_token1_mint_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[4].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_skim_invalid_recipient_token0_ata_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[5].pubkey = ctx.user_token1[0];
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_skim_invalid_recipient_token1_ata_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    // Interact
    let mut ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    ix.accounts[6].pubkey = ctx.user_token0[0];
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_skim_missing_payer_signature_reverted_when_creating_ata() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let recipient = Keypair::new();
    svm.airdrop(&recipient.pubkey(), 1_000_000_000).unwrap();

    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token0[1],
        &ctx.vault0,
        &ctx.users[1],
        50_000,
    );
    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token1[1],
        &ctx.vault1,
        &ctx.users[1],
        25_000,
    );

    // Interact
    let mut ix = create_skim_ix(&ctx, &recipient.pubkey());
    ix.accounts[0].pubkey = ctx.users[0].pubkey();
    ix.accounts[0].is_signer = false;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::MissingRequiredSignature)
    );
}

#[test]
fn test_skim_recipient_token0_owner_mismatch_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let wrong_owner_token0 = svm.get_account(&ctx.user_token0[1]).unwrap();
    seed_account(&mut svm, ctx.user_token0[0], wrong_owner_token0);

    // Interact
    let ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::Custom(0))
    );
}

#[test]
fn test_skim_recipient_token1_mint_mismatch_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let wrong_mint_token1 = svm.get_account(&ctx.user_token0[0]).unwrap();
    seed_account(&mut svm, ctx.user_token1[0], wrong_mint_token1);

    // Interact
    let ix = create_skim_ix(&ctx, &ctx.users[0].pubkey());
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}
