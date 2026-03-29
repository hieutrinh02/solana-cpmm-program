use borsh::to_vec;
use solana_sdk::{
    account::Account, clock::Clock, instruction::InstructionError, pubkey::Pubkey,
    signature::Signer, transaction::TransactionError,
};

use solana_cpmm_program::{
    constants::MINIMUM_LIQUIDITY,
    instructions::lib::get_ata,
    math::sqrt_u128,
};

mod helper;

use helper::{
    bootstrap_pair, compute_swap_amount_out, create_add_liquidity_ix, create_ata,
    create_remove_liquidity_ix, create_swap_ix, expected_initial_user_lp, get_mint_supply,
    get_pair_state, get_token_balance, seed_account, send_signed_transaction, setup,
    token_transfer_ix, transfer_tokens, INITIAL_LIQUIDITY, INITIAL_USER_FUNDS,
};

#[test]
fn test_remove_liquidity() {
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

    let mut clock = svm.get_sysvar::<Clock>();
    clock.unix_timestamp = (initial_timestamp + time_elapsed) as i64;
    svm.set_sysvar::<Clock>(&clock);

    // Interact
    let remove_ix = create_remove_liquidity_ix(&ctx, 0, expected_initial_user_lp(), 1, 1);
    send_signed_transaction(&mut svm, &ctx.users[0], &[remove_ix], &[&ctx.users[0]]).unwrap();

    // Assert
    let pair_state = get_pair_state(&svm, &ctx.pair);
    let payer_lp = get_ata(&ctx.users[0].pubkey(), &ctx.lp_mint);
    let user0_token0_balance = get_token_balance(&svm, &ctx.user_token0[0]);
    let user0_token1_balance = get_token_balance(&svm, &ctx.user_token1[0]);
    let vault0_balance = get_token_balance(&svm, &ctx.vault0);
    let vault1_balance = get_token_balance(&svm, &ctx.vault1);
    let payer_lp_balance = get_token_balance(&svm, &payer_lp);
    let locked_lp_balance = get_token_balance(&svm, &ctx.locked_lp);
    let lp_supply = get_mint_supply(&svm, &ctx.lp_mint);
    let expected_price0_cumulative = (1u128 << 64) * time_elapsed as u128;
    let expected_price1_cumulative = (1u128 << 64) * time_elapsed as u128;

    assert_eq!(user0_token0_balance, INITIAL_USER_FUNDS - MINIMUM_LIQUIDITY);
    assert_eq!(user0_token1_balance, INITIAL_USER_FUNDS - MINIMUM_LIQUIDITY);
    assert_eq!(vault0_balance, MINIMUM_LIQUIDITY);
    assert_eq!(vault1_balance, MINIMUM_LIQUIDITY);
    assert_eq!(payer_lp_balance, 0);
    assert_eq!(locked_lp_balance, MINIMUM_LIQUIDITY);
    assert_eq!(lp_supply, MINIMUM_LIQUIDITY);
    assert_eq!(pair_state.reserve0, MINIMUM_LIQUIDITY);
    assert_eq!(pair_state.reserve1, MINIMUM_LIQUIDITY);
    assert_eq!(
        pair_state.k_last,
        (MINIMUM_LIQUIDITY as u128) * (MINIMUM_LIQUIDITY as u128)
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
fn test_remove_liquidity_mints_protocol_fee_before_burn() {
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
    let swap_ix = create_swap_ix(&ctx, 1, 0, amount1_out);
    send_signed_transaction(
        &mut svm,
        &ctx.users[1],
        &[transfer_ix, swap_ix],
        &[&ctx.users[1]],
    )
    .unwrap();

    let pair_before = get_pair_state(&svm, &ctx.pair);
    let total_supply_before = get_mint_supply(&svm, &ctx.lp_mint);
    let root_k = sqrt_u128((pair_before.reserve0 as u128) * (pair_before.reserve1 as u128));
    let root_k_last = sqrt_u128(pair_before.k_last);
    let expected_fee_lp =
        ((total_supply_before as u128) * (root_k - root_k_last)) / ((root_k * 5) + root_k_last);
    assert!(expected_fee_lp > 0);

    let liquidity_to_burn = expected_initial_user_lp() / 10;
    let total_supply_after_fee = total_supply_before + expected_fee_lp as u64;
    let expected_amount0 = ((liquidity_to_burn as u128) * (pair_before.reserve0 as u128)
        / (total_supply_after_fee as u128)) as u64;
    let expected_amount1 = ((liquidity_to_burn as u128) * (pair_before.reserve1 as u128)
        / (total_supply_after_fee as u128)) as u64;
    let user0_lp_before = get_token_balance(&svm, &get_ata(&ctx.users[0].pubkey(), &ctx.lp_mint));
    let user0_token0_before = get_token_balance(&svm, &ctx.user_token0[0]);
    let user0_token1_before = get_token_balance(&svm, &ctx.user_token1[0]);

    // Interact
    let remove_ix = create_remove_liquidity_ix(&ctx, 0, liquidity_to_burn, 1, 1);
    send_signed_transaction(&mut svm, &ctx.users[0], &[remove_ix], &[&ctx.users[0]]).unwrap();

    // Assert
    let pair_after = get_pair_state(&svm, &ctx.pair);
    let user0_lp = get_ata(&ctx.users[0].pubkey(), &ctx.lp_mint);
    let user0_lp_after = get_token_balance(&svm, &user0_lp);
    let admin_lp_balance = get_token_balance(&svm, &ctx.admin_lp);
    let total_supply_after = get_mint_supply(&svm, &ctx.lp_mint);

    assert_eq!(admin_lp_balance, expected_fee_lp as u64);
    assert_eq!(user0_lp_after, user0_lp_before - liquidity_to_burn);
    assert_eq!(
        get_token_balance(&svm, &ctx.user_token0[0]),
        user0_token0_before + expected_amount0
    );
    assert_eq!(
        get_token_balance(&svm, &ctx.user_token1[0]),
        user0_token1_before + expected_amount1
    );
    assert_eq!(
        total_supply_after,
        total_supply_before + expected_fee_lp as u64 - liquidity_to_burn
    );
    assert_eq!(
        pair_after.k_last,
        (pair_after.reserve0 as u128) * (pair_after.reserve1 as u128)
    );
}

#[test]
fn test_remove_liquidity_missing_signer_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
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
fn test_remove_liquidity_invalid_system_program_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[14].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_remove_liquidity_invalid_token_program_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[12].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_remove_liquidity_invalid_ata_program_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[13].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_remove_liquidity_invalid_rent_sysvar_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[15].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_remove_liquidity_zero_liquidity_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    // Interact
    let ix = create_remove_liquidity_ix(&ctx, 0, 0, 1, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_pair_owner_mismatch_reverted() {
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
    let ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_remove_liquidity_pair_not_initialized_reverted() {
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
    let ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::UninitializedAccount)
    );
}

#[test]
fn test_remove_liquidity_pair_factory_mismatch_reverted() {
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
    let ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_remove_liquidity_invalid_pair_pda_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[1].pubkey = fake_pair;
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidSeeds)
    );
}

#[test]
fn test_remove_liquidity_invalid_vault_linkage_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[6].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_invalid_lp_mint_linkage_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[8].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_invalid_token0_mint_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[2].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_invalid_token1_mint_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[3].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_wrong_payer_token0_account_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[4].pubkey = ctx.user_token0[1];
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_wrong_payer_token1_mint_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[5].pubkey = ctx.user_token0[0];
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_lp_mint_owner_mismatch_reverted() {
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

    let mut lp_mint_account = svm.get_account(&ctx.lp_mint).unwrap();
    lp_mint_account.owner = Pubkey::new_unique();
    seed_account(&mut svm, ctx.lp_mint, lp_mint_account);

    // Interact
    let ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_remove_liquidity_invalid_fee_recipient_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[10].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_zero_vault_balance_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    // Interact
    let ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_remove_liquidity_zero_lp_supply_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_pair(&mut svm, &ctx);

    create_ata(
        &mut svm,
        &ctx.users[0],
        &ctx.users[0].pubkey(),
        &ctx.lp_mint,
    );
    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token0[1],
        &ctx.vault0,
        &ctx.users[1],
        1,
    );
    transfer_tokens(
        &mut svm,
        &ctx.users[1],
        &ctx.user_token1[1],
        &ctx.vault1,
        &ctx.users[1],
        1,
    );

    // Interact
    let ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_remove_liquidity_cannot_burn_entire_supply_reverted() {
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
    let ix = create_remove_liquidity_ix(&ctx, 0, get_mint_supply(&svm, &ctx.lp_mint), 1, 1);
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_withdrawal_below_min_reverted() {
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
    let ix = create_remove_liquidity_ix(
        &ctx,
        0,
        expected_initial_user_lp(),
        INITIAL_LIQUIDITY,
        INITIAL_LIQUIDITY,
    );
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_remove_liquidity_invalid_clock_sysvar_reverted() {
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
    let mut ix = create_remove_liquidity_ix(&ctx, 0, 1, 1, 1);
    ix.accounts[16].pubkey = Pubkey::new_unique();
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}
