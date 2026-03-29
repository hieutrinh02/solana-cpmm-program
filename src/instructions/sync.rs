use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
};
use solana_program_pack::Pack;
use spl_token_interface::state::Account as TokenAccount;

use super::lib::{
    assert_token_account, derive_factory_pda, derive_pair_pda, get_ata, update_reserves_and_twap,
};
use crate::state::PairState;

/// Synchronizes cached reserves with actual vault balances.
pub fn sync<'a>(program_id: &Pubkey, accounts: &'a [AccountInfo<'a>]) -> ProgramResult {
    // Accounts
    // 0) pair_pda (writable) - PairState (program-owned) + authority
    // 1) vault0_ata (read-only) - ATA(pair, mint0)
    // 2) vault1_ata (read-only) - ATA(pair, mint1)
    // 3) clock_sysvar (read-only)

    let accounts_iter = &mut accounts.iter();

    let pair = next_account_info(accounts_iter)?;
    let vault0 = next_account_info(accounts_iter)?;
    let vault1 = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    // Pair state ownership checks.
    if pair.owner != program_id {
        msg!("pair owner mismatch");
        return Err(ProgramError::IllegalOwner);
    }

    // Pair state checks.
    let mut pair_state = PairState::try_from_slice(&pair.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;
    if !pair_state.is_initialized {
        msg!("pair not initialized");
        return Err(ProgramError::UninitializedAccount);
    }

    // PDA ownership checks.
    let (expected_factory, _) = derive_factory_pda(program_id);
    if pair_state.factory != expected_factory {
        msg!("pair does not belong to factory");
        return Err(ProgramError::InvalidAccountData);
    }
    let (expected_pair, _) = derive_pair_pda(
        program_id,
        &expected_factory,
        &pair_state.token0_mint,
        &pair_state.token1_mint,
    );
    if *pair.key != expected_pair {
        msg!("invalid pair pda");
        return Err(ProgramError::InvalidSeeds);
    }

    // State linkage checks.
    if *vault0.key != pair_state.vault0 {
        msg!("invalid vault0");
        return Err(ProgramError::InvalidArgument);
    }
    if *vault1.key != pair_state.vault1 {
        msg!("invalid vault1");
        return Err(ProgramError::InvalidArgument);
    }

    // Token account and ATA checks.
    assert_token_account(vault0, pair.key, &pair_state.token0_mint)?;
    assert_token_account(vault1, pair.key, &pair_state.token1_mint)?;

    let expected_vault0 = get_ata(pair.key, &pair_state.token0_mint);
    let expected_vault1 = get_ata(pair.key, &pair_state.token1_mint);
    if *vault0.key != expected_vault0 {
        msg!("invalid vault0 ata");
        return Err(ProgramError::InvalidArgument);
    }
    if *vault1.key != expected_vault1 {
        msg!("invalid vault1 ata");
        return Err(ProgramError::InvalidArgument);
    }

    // Balance observation.
    let vault0_state = TokenAccount::unpack(&vault0.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let vault1_state = TokenAccount::unpack(&vault1.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;

    // State writeback.
    update_reserves_and_twap(
        &mut pair_state,
        clock_sysvar,
        vault0_state.amount,
        vault1_state.amount,
    )?;

    pair_state
        .serialize(&mut &mut pair.data.borrow_mut()[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;

    Ok(())
}
