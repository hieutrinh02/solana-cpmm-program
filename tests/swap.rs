use borsh::to_vec;
use solana_sdk::{
    account::Account, clock::Clock, instruction::InstructionError, pubkey::Pubkey,
    signature::Signer, transaction::TransactionError,
};

mod helper;

use helper::{
    bootstrap_pair, compute_swap_amount_out, create_add_liquidity_ix, create_swap_ix,
    get_pair_state, get_token_balance, seed_account, send_signed_transaction, setup,
    token_transfer_ix, INITIAL_LIQUIDITY,
};

#[test]
fn test_swap() {
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

    let amount_in = 1_000_000;
    let amount1_out = compute_swap_amount_out(amount_in, INITIAL_LIQUIDITY, INITIAL_LIQUIDITY);

    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp = (initial_timestamp + time_elapsed) as i64;
    svm.set_sysvar::<Clock>(&clock);

    let user0_before = get_token_balance(&svm, &ctx.user_token0[1]);
    let user1_before = get_token_balance(&svm, &ctx.user_token1[1]);

    // Interact
    let transfer_ix = token_transfer_ix(
        &ctx.user_token0[1],
        &ctx.vault0,
        &ctx.users[1].pubkey(),
        amount_in,
    );
    let swap_ix = create_swap_ix(&ctx, 1, 0, amount1_out);

    send_signed_transaction(
        &mut svm,
        &ctx.users[1],
        &[transfer_ix, swap_ix],
        &[&ctx.users[1]],
    )
    .unwrap();

    // Assert
    let pair_state = get_pair_state(&svm, &ctx.pair);
    let user1_token0_balance = get_token_balance(&svm, &ctx.user_token0[1]);
    let user1_token1_balance = get_token_balance(&svm, &ctx.user_token1[1]);
    let vault0_balance = get_token_balance(&svm, &ctx.vault0);
    let vault1_balance = get_token_balance(&svm, &ctx.vault1);
    let expected_price0_cumulative = (1u128 << 64) * time_elapsed as u128;
    let expected_price1_cumulative = (1u128 << 64) * time_elapsed as u128;

    assert_eq!(user1_token0_balance, user0_before - amount_in);
    assert_eq!(user1_token1_balance, user1_before + amount1_out);
    assert_eq!(vault0_balance, INITIAL_LIQUIDITY + amount_in);
    assert_eq!(vault1_balance, INITIAL_LIQUIDITY - amount1_out);
    assert_eq!(pair_state.reserve0, INITIAL_LIQUIDITY + amount_in);
    assert_eq!(pair_state.reserve1, INITIAL_LIQUIDITY - amount1_out);
    assert_eq!(
        pair_state.k_last,
        (INITIAL_LIQUIDITY as u128) * (INITIAL_LIQUIDITY as u128)
    );
    assert_eq!(
        pair_state.price0_cumulative_last,
        expected_price0_cumulative
    );
    assert_eq!(
        pair_state.price1_cumulative_last,
        expected_price1_cumulative
    );
    assert_eq!(
        pair_state.block_timestamp_last,
        initial_timestamp + time_elapsed
    );
}

#[test]
fn test_swap_missing_signer_reverted() {
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
    let mut ix = create_swap_ix(&ctx, 1, 0, 1);
    ix.accounts[0].is_signer = false;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::MissingRequiredSignature)
    );
}

#[test]
fn test_swap_invalid_token_program_reverted() {
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
    let mut ix = create_swap_ix(&ctx, 1, 0, 1);
    ix.accounts[6].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_swap_zero_output_reverted() {
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
    let ix = create_swap_ix(&ctx, 1, 0, 0);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_swap_pair_owner_mismatch_reverted() {
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
    let ix = create_swap_ix(&ctx, 1, 0, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_swap_pair_not_initialized_reverted() {
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
    let ix = create_swap_ix(&ctx, 1, 0, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::UninitializedAccount)
    );
}

#[test]
fn test_swap_pair_factory_mismatch_reverted() {
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
    let ix = create_swap_ix(&ctx, 1, 0, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_swap_invalid_pair_pda_reverted() {
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
    let mut ix = create_swap_ix(&ctx, 1, 0, 1);
    ix.accounts[1].pubkey = fake_pair;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidSeeds)
    );
}

#[test]
fn test_swap_invalid_vault_linkage_reverted() {
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
    let mut ix = create_swap_ix(&ctx, 1, 0, 1);
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
fn test_swap_wrong_user_token0_account_reverted() {
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
    let mut ix = create_swap_ix(&ctx, 1, 0, 1);
    ix.accounts[2].pubkey = ctx.user_token0[0];
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_swap_wrong_user_token1_mint_reverted() {
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
    let mut ix = create_swap_ix(&ctx, 1, 0, 1);
    ix.accounts[3].pubkey = ctx.user_token0[1];
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_swap_zero_reserve_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    // Interact
    let ix = create_swap_ix(&ctx, 1, 0, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_swap_output_exceeds_reserve_reverted() {
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
    let ix = create_swap_ix(&ctx, 1, 0, INITIAL_LIQUIDITY);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_swap_missing_input_amount_reverted() {
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
    let ix = create_swap_ix(&ctx, 1, 0, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[1], &[ix], &[&ctx.users[1]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_swap_k_invariant_violation_reverted() {
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
    let transfer_ix =
        token_transfer_ix(&ctx.user_token0[1], &ctx.vault0, &ctx.users[1].pubkey(), 1);
    let swap_ix = create_swap_ix(&ctx, 1, 0, 1_000);
    let err = send_signed_transaction(
        &mut svm,
        &ctx.users[1],
        &[transfer_ix, swap_ix],
        &[&ctx.users[1]],
    )
    .unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(1, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_swap_invalid_clock_sysvar_reverted() {
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

    let amount_in = 1_000_000;
    let amount1_out = compute_swap_amount_out(amount_in, INITIAL_LIQUIDITY, INITIAL_LIQUIDITY);
    let transfer_ix = token_transfer_ix(
        &ctx.user_token0[1],
        &ctx.vault0,
        &ctx.users[1].pubkey(),
        amount_in,
    );
    let mut swap_ix = create_swap_ix(&ctx, 1, 0, amount1_out);
    swap_ix.accounts[7].pubkey = Pubkey::new_unique();

    // Interact
    let err = send_signed_transaction(
        &mut svm,
        &ctx.users[1],
        &[transfer_ix, swap_ix],
        &[&ctx.users[1]],
    )
    .unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(1, InstructionError::InvalidAccountData)
    );
}
