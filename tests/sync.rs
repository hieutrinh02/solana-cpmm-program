use borsh::to_vec;
use solana_sdk::{
    account::Account, clock::Clock, instruction::InstructionError, pubkey::Pubkey,
    transaction::TransactionError,
};

mod helper;

use helper::{
    bootstrap_pair, create_add_liquidity_ix, create_sync_ix, get_pair_state, seed_account,
    send_signed_transaction, setup, transfer_tokens, INITIAL_LIQUIDITY,
};

#[test]
fn test_sync() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);
    let initial_timestamp = 5u64;
    let time_elapsed = 10u64;

    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp = initial_timestamp as i64;
    svm.set_sysvar::<Clock>(&clock);

    let add_ix = create_add_liquidity_ix(
        &ctx,
        0,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    send_signed_transaction(&mut svm, &ctx.users[0], &[add_ix], &[&ctx.users[0]]).unwrap();

    let extra0 = 123_456;
    let extra1 = 654_321;
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

    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp = (initial_timestamp + time_elapsed) as i64;
    svm.set_sysvar::<Clock>(&clock);

    // Interact
    let sync_ix = create_sync_ix(&ctx);
    send_signed_transaction(&mut svm, &ctx.users[1], &[sync_ix], &[&ctx.users[1]]).unwrap();

    // Assert
    let pair_state = get_pair_state(&svm, &ctx.pair);
    let expected_price_x64 = 1u128 << 64;
    let expected_cumulative = expected_price_x64 * time_elapsed as u128;
    assert_eq!(pair_state.reserve0, INITIAL_LIQUIDITY + extra0);
    assert_eq!(pair_state.reserve1, INITIAL_LIQUIDITY + extra1);
    assert_eq!(pair_state.price0_cumulative_last, expected_cumulative);
    assert_eq!(pair_state.price1_cumulative_last, expected_cumulative);
    assert_eq!(
        pair_state.block_timestamp_last,
        initial_timestamp + time_elapsed
    );
}

#[test]
fn test_sync_pair_owner_mismatch_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    let mut pair_account = svm.get_account(&ctx.pair).unwrap();
    pair_account.owner = Pubkey::new_unique();
    seed_account(&mut svm, ctx.pair, pair_account);

    // Interact
    let ix = create_sync_ix(&ctx);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_sync_pair_not_initialized_reverted() {
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
    let ix = create_sync_ix(&ctx);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::UninitializedAccount)
    );
}

#[test]
fn test_sync_pair_factory_mismatch_reverted() {
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
    let ix = create_sync_ix(&ctx);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_sync_invalid_pair_pda_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

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
    let mut ix = create_sync_ix(&ctx);
    ix.accounts[0].pubkey = fake_pair;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidSeeds)
    );
}

#[test]
fn test_sync_invalid_vault0_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    // Interact
    let mut ix = create_sync_ix(&ctx);
    ix.accounts[1].pubkey = ctx.vault1;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_sync_invalid_vault1_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    // Interact
    let mut ix = create_sync_ix(&ctx);
    ix.accounts[2].pubkey = ctx.vault0;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_sync_invalid_clock_sysvar_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    // Interact
    let mut ix = create_sync_ix(&ctx);
    ix.accounts[3].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}
