use solana_sdk::{
    instruction::InstructionError, pubkey::Pubkey, signature::Signer, transaction::TransactionError,
};

mod helper;

use helper::{
    create_init_factory_ix, get_factory_state, send_admin_transaction, send_signed_transaction,
    set_instruction_signer, setup,
};

#[test]
fn test_init_factory() {
    // Setup
    let (mut svm, ctx) = setup();

    // Interact
    let result = send_admin_transaction(&mut svm, ctx.admin, &[create_init_factory_ix(&ctx)]);
    assert!(result.is_ok());

    // Assert
    let factory_state = get_factory_state(&svm, &ctx.factory);
    assert!(factory_state.is_initialized);
    assert_eq!(factory_state.pair_count, 0);
    assert_eq!(svm.get_account(&ctx.factory).unwrap().owner, ctx.program_id);
}

#[test]
fn test_init_factory_missing_signer_reverted() {
    // Setup
    let (mut svm, ctx) = setup();

    // Interact
    let mut ix = create_init_factory_ix(&ctx);
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
fn test_init_factory_non_admin_reverted() {
    // Setup
    let (mut svm, ctx) = setup();

    // Interact
    let mut ix = create_init_factory_ix(&ctx);
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
fn test_init_factory_invalid_system_program_reverted() {
    // Setup
    let (mut svm, ctx) = setup();

    // Interact
    let mut ix = create_init_factory_ix(&ctx);
    ix.accounts[2].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::IncorrectProgramId)
    );
}

#[test]
fn test_init_factory_invalid_factory_pda_reverted() {
    // Setup
    let (mut svm, ctx) = setup();

    // Interact
    let mut ix = create_init_factory_ix(&ctx);
    ix.accounts[1].pubkey = Pubkey::new_unique();
    let err = send_admin_transaction(&mut svm, ctx.admin, &[ix]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::InvalidSeeds)
    );
}

#[test]
fn test_init_factory_already_initialized_reverted() {
    // Setup
    let (mut svm, ctx) = setup();

    send_admin_transaction(&mut svm, ctx.admin, &[create_init_factory_ix(&ctx)]).unwrap();

    // Interact
    let err =
        send_admin_transaction(&mut svm, ctx.admin, &[create_init_factory_ix(&ctx)]).unwrap_err();

    // Assert
    assert_eq!(
        err.err,
        TransactionError::InstructionError(0, InstructionError::AccountAlreadyInitialized)
    );
}
