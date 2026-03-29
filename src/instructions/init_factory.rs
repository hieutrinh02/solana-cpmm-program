use borsh::BorshSerialize;
#[allow(deprecated)]
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_program,
    sysvar::Sysvar,
};

use super::lib::{create_or_claim_pda_account, derive_factory_pda};
use crate::{
    constants::{FACTORY_SEED, PROGRAM_ADMIN_AUTHORITY},
    state::FactoryState,
};

/// Initializes the singleton factory account for the program.
///
/// The payer must match [`PROGRAM_ADMIN_AUTHORITY`].
pub fn init_factory<'a>(program_id: &Pubkey, accounts: &'a [AccountInfo<'a>]) -> ProgramResult {
    // Accounts
    // 0) payer (signer, writable)
    // 1) factory_pda (writable) - FactoryState (program-owned)
    // 2) system_program (read-only)
    let accounts_iter = &mut accounts.iter();

    let payer = next_account_info(accounts_iter)?;
    let factory = next_account_info(accounts_iter)?;
    let sys_program = next_account_info(accounts_iter)?;

    // Authority checks.
    if !payer.is_signer {
        msg!("payer must be a signer");
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *payer.key != PROGRAM_ADMIN_AUTHORITY {
        msg!("payer is not authorized to initialize factory");
        return Err(ProgramError::IllegalOwner);
    }

    // Static program checks.
    if *sys_program.key != system_program::id() {
        msg!("invalid system program");
        return Err(ProgramError::IncorrectProgramId);
    }

    // PDA checks.
    let (expected_factory, factory_bump) = derive_factory_pda(program_id);
    if expected_factory != *factory.key {
        msg!("invalid factory PDA");
        return Err(ProgramError::InvalidSeeds);
    }

    // Account creation.
    let space = FactoryState::LEN;
    let lamports = Rent::get()?.minimum_balance(space);

    create_or_claim_pda_account(
        payer,
        factory,
        program_id,
        lamports,
        space as u64,
        sys_program,
        &[FACTORY_SEED, &[factory_bump]],
    )?;

    // State writeback.
    let factory_state = FactoryState {
        is_initialized: true,
        pair_count: 0,
    };

    factory_state.serialize(&mut &mut factory.data.borrow_mut()[..])?;

    Ok(())
}
