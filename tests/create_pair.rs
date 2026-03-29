use litesvm_token::{get_spl_account, spl_token::state::Mint};
use solana_sdk::{
    instruction::InstructionError, pubkey::Pubkey, signature::Signer, transaction::TransactionError,
};
use spl_token_interface::id as spl_token_program_id;

use solana_cpmm_program::{constants::LP_DECIMALS, state::FactoryState};

mod helper;

use helper::{
    bootstrap_factory, create_create_pair_ix, get_factory_state, get_pair_state,
    seed_factory_account, send_admin_transaction, send_signed_transaction, set_instruction_signer,
    setup,
};

#[test]
fn test_create_pair() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let result = send_admin_transaction(&mut svm, ctx.admin, &[create_create_pair_ix(&ctx)]);
    assert!(result.is_ok());

    // Assert
    let factory_state = get_factory_state(&svm, &ctx.factory);
    let pair_state = get_pair_state(&svm, &ctx.pair);
    let lp_mint_state: Mint = get_spl_account(&svm, &ctx.lp_mint).unwrap();

    assert_eq!(factory_state.pair_count, 1);
    assert!(pair_state.is_initialized);
    assert_eq!(pair_state.factory, ctx.factory);
    assert_eq!(pair_state.token0_mint, ctx.mint0);
    assert_eq!(pair_state.token1_mint, ctx.mint1);
    assert_eq!(pair_state.vault0, ctx.vault0);
    assert_eq!(pair_state.vault1, ctx.vault1);
    assert_eq!(pair_state.lp_mint, ctx.lp_mint);
    assert_eq!(pair_state.reserve0, 0);
    assert_eq!(pair_state.reserve1, 0);
    assert_eq!(pair_state.k_last, 0);
    assert_eq!(pair_state.price0_cumulative_last, 0);
    assert_eq!(pair_state.price1_cumulative_last, 0);
    assert_eq!(pair_state.block_timestamp_last, 0);
    assert!(lp_mint_state.is_initialized);
    assert_eq!(
        Option::<Pubkey>::from(lp_mint_state.mint_authority),
        Some(ctx.pair)
    );
    assert_eq!(Option::<Pubkey>::from(lp_mint_state.freeze_authority), None);
    assert_eq!(lp_mint_state.decimals, LP_DECIMALS);
    assert_eq!(lp_mint_state.supply, 0);
    assert_eq!(svm.get_account(&ctx.pair).unwrap().owner, ctx.program_id);
    assert_eq!(
        svm.get_account(&ctx.vault0).unwrap().owner,
        Pubkey::from(spl_token_program_id().to_bytes())
    );
    assert_eq!(
        svm.get_account(&ctx.vault1).unwrap().owner,
        Pubkey::from(spl_token_program_id().to_bytes())
    );
    assert_eq!(
        svm.get_account(&ctx.lp_mint).unwrap().owner,
        Pubkey::from(spl_token_program_id().to_bytes())
    );
}

#[test]
fn test_create_pair_missing_signer_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
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
fn test_create_pair_non_admin_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    set_instruction_signer(&mut ix, 0, ctx.users[0].pubkey());
    let err =
        send_signed_transaction(&mut svm, &ctx.users[0], &[ix], &[&ctx.users[0]]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_create_pair_invalid_system_program_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[10].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_create_pair_invalid_token_program_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[8].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_create_pair_invalid_ata_program_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[9].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_create_pair_invalid_rent_sysvar_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[11].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidAccountData)
    );
}

#[test]
fn test_create_pair_identical_mints_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[4].pubkey = ctx.mint_a;
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_create_pair_invalid_factory_pda_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[1].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidSeeds)
    );
}

#[test]
fn test_create_pair_factory_owner_mismatch_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    seed_factory_account(
        &mut svm,
        &ctx,
        Pubkey::new_unique(),
        FactoryState {
            is_initialized: true,
            pair_count: 0,
        },
    );

    // Interact
    let err =
        send_admin_transaction(&mut svm, ctx.admin, &[create_create_pair_ix(&ctx)]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_create_pair_factory_not_initialized_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    seed_factory_account(
        &mut svm,
        &ctx,
        ctx.program_id,
        FactoryState {
            is_initialized: false,
            pair_count: 0,
        },
    );

    // Interact
    let err =
        send_admin_transaction(&mut svm, ctx.admin, &[create_create_pair_ix(&ctx)]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::UninitializedAccount)
    );
}

#[test]
fn test_create_pair_mint_a_wrong_owner_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[3].pubkey = ctx.factory;
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_create_pair_mint_b_wrong_owner_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[4].pubkey = ctx.factory;
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IllegalOwner)
    );
}

#[test]
fn test_create_pair_invalid_pair_pda_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[2].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidSeeds)
    );
}

#[test]
fn test_create_pair_invalid_vault_atas_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[5].pubkey = Pubkey::new_unique();
    ix.accounts[6].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidArgument)
    );
}

#[test]
fn test_create_pair_invalid_lp_mint_pda_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);

    // Interact
    let mut ix = create_create_pair_ix(&ctx);
    ix.accounts[7].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidSeeds)
    );
}

#[test]
fn test_create_pair_pair_already_initialized_reverted() {
    // Setup
    let (mut svm, ctx) = setup();
    bootstrap_factory(&mut svm, &ctx);
    send_admin_transaction(&mut svm, ctx.admin, &[create_create_pair_ix(&ctx)]).unwrap();

    // Interact
    let err =
        send_admin_transaction(&mut svm, ctx.admin, &[create_create_pair_ix(&ctx)]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::AccountAlreadyInitialized)
    );
}
