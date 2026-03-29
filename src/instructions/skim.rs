use borsh::BorshDeserialize;
#[allow(deprecated)]
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    system_program, sysvar,
};
use solana_program_pack::Pack;
use spl_associated_token_account_interface;
use spl_token_interface::{self, state::Account as TokenAccount};

use super::lib::{
    assert_token_account, create_ata_if_needed, derive_factory_pda, derive_pair_pda, get_ata,
    transfer_signed, SplProgramAccounts,
};
use crate::{constants::PAIR_SEED_PREFIX, state::PairState};

/// Transfers any excess vault balances above cached reserves to the provided
/// recipient token accounts without updating reserves.
pub fn skim<'a>(program_id: &Pubkey, accounts: &'a [AccountInfo<'a>]) -> ProgramResult {
    // Accounts
    // 0) payer (signer, writable) - funds recipient ATA creation if missing
    // 1) recipient (read-only)
    // 2) pair_pda (read-only) - PairState (program-owned) + authority
    // 3) token0_mint (read-only)
    // 4) token1_mint (read-only)
    // 5) recipient_token0_ata (writable)
    // 6) recipient_token1_ata (writable)
    // 7) vault0_ata (writable) - ATA(pair, mint0)
    // 8) vault1_ata (writable) - ATA(pair, mint1)
    // 9) token_program (read-only)
    // 10) ata_program (read-only)
    // 11) system_program (read-only)
    // 12) rent_sysvar (read-only)

    let accounts_iter = &mut accounts.iter();

    let payer = next_account_info(accounts_iter)?;
    let recipient = next_account_info(accounts_iter)?;
    let pair = next_account_info(accounts_iter)?;
    let token0_mint = next_account_info(accounts_iter)?;
    let token1_mint = next_account_info(accounts_iter)?;
    let recipient_token0 = next_account_info(accounts_iter)?;
    let recipient_token1 = next_account_info(accounts_iter)?;
    let vault0 = next_account_info(accounts_iter)?;
    let vault1 = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let ata_program = next_account_info(accounts_iter)?;
    let sys_program = next_account_info(accounts_iter)?;
    let rent_sysvar = next_account_info(accounts_iter)?;
    let spl_programs = SplProgramAccounts {
        token_program,
        ata_program,
        system_program: sys_program,
        rent_sysvar,
    };

    // Static program and sysvar checks.
    if *sys_program.key != system_program::id() {
        msg!("invalid system program");
        return Err(ProgramError::IncorrectProgramId);
    }
    if token_program.key.to_bytes() != spl_token_interface::id().to_bytes() {
        msg!("invalid token program");
        return Err(ProgramError::IncorrectProgramId);
    }
    if ata_program.key.to_bytes()
        != spl_associated_token_account_interface::program::id().to_bytes()
    {
        msg!("invalid ata program");
        return Err(ProgramError::IncorrectProgramId);
    }
    if *rent_sysvar.key != sysvar::rent::id() {
        msg!("invalid rent sysvar");
        return Err(ProgramError::InvalidAccountData);
    }

    // Pair state ownership checks.
    if pair.owner != program_id {
        msg!("pair owner mismatch");
        return Err(ProgramError::IllegalOwner);
    }

    // Pair state checks.
    let pair_state = PairState::try_from_slice(&pair.data.borrow())
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
    let (expected_pair, pair_bump) = derive_pair_pda(
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

    if *token0_mint.key != pair_state.token0_mint {
        msg!("invalid token0 mint");
        return Err(ProgramError::InvalidArgument);
    }
    if *token1_mint.key != pair_state.token1_mint {
        msg!("invalid token1 mint");
        return Err(ProgramError::InvalidArgument);
    }

    let expected_recipient_token0 = get_ata(recipient.key, &pair_state.token0_mint);
    let expected_recipient_token1 = get_ata(recipient.key, &pair_state.token1_mint);
    let expected_vault0 = get_ata(pair.key, &pair_state.token0_mint);
    let expected_vault1 = get_ata(pair.key, &pair_state.token1_mint);

    if *recipient_token0.key != expected_recipient_token0 {
        msg!("invalid recipient token0 ata");
        return Err(ProgramError::InvalidArgument);
    }
    if *recipient_token1.key != expected_recipient_token1 {
        msg!("invalid recipient token1 ata");
        return Err(ProgramError::InvalidArgument);
    }
    if *vault0.key != expected_vault0 {
        msg!("invalid vault0 ata");
        return Err(ProgramError::InvalidArgument);
    }
    if *vault1.key != expected_vault1 {
        msg!("invalid vault1 ata");
        return Err(ProgramError::InvalidArgument);
    }

    let recipient_token0_ata_ready = match TokenAccount::unpack(&recipient_token0.data.borrow()) {
        Ok(account_state) => {
            if Pubkey::from(account_state.owner.to_bytes()) == *recipient.key
                && Pubkey::from(account_state.mint.to_bytes()) == pair_state.token0_mint
            {
                true
            } else {
                false
            }
        }
        Err(_) => false,
    };

    let recipient_token1_ata_ready = match TokenAccount::unpack(&recipient_token1.data.borrow()) {
        Ok(account_state) => {
            if Pubkey::from(account_state.owner.to_bytes()) == *recipient.key
                && Pubkey::from(account_state.mint.to_bytes()) == pair_state.token1_mint
            {
                true
            } else {
                false
            }
        }
        Err(_) => false,
    };

    if (!recipient_token0_ata_ready || !recipient_token1_ata_ready) && !payer.is_signer {
        msg!("payer must sign when creating recipient ata");
        return Err(ProgramError::MissingRequiredSignature);
    }

    if !recipient_token0_ata_ready {
        create_ata_if_needed(
            payer,
            recipient_token0,
            recipient,
            token0_mint,
            &spl_programs,
        )?;
    }
    if !recipient_token1_ata_ready {
        create_ata_if_needed(
            payer,
            recipient_token1,
            recipient,
            token1_mint,
            &spl_programs,
        )?;
    }

    // Token account and ATA checks.
    assert_token_account(recipient_token0, recipient.key, &pair_state.token0_mint)?;
    assert_token_account(recipient_token1, recipient.key, &pair_state.token1_mint)?;
    assert_token_account(vault0, pair.key, &pair_state.token0_mint)?;
    assert_token_account(vault1, pair.key, &pair_state.token1_mint)?;

    // Business logic setup.
    let factory_key = pair_state.factory;
    let token0_mint = pair_state.token0_mint;
    let token1_mint = pair_state.token1_mint;
    let pair_signer_seeds: &[&[u8]] = &[
        PAIR_SEED_PREFIX,
        factory_key.as_ref(),
        token0_mint.as_ref(),
        token1_mint.as_ref(),
        &[pair_bump],
    ];

    // Balance observation.
    let vault0_state = TokenAccount::unpack(&vault0.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let vault1_state = TokenAccount::unpack(&vault1.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;

    let excess0 = vault0_state.amount.saturating_sub(pair_state.reserve0);
    let excess1 = vault1_state.amount.saturating_sub(pair_state.reserve1);

    // Transfers.
    if excess0 > 0 {
        transfer_signed(
            token_program,
            vault0,
            recipient_token0,
            pair,
            excess0,
            pair_signer_seeds,
        )?;
    }
    if excess1 > 0 {
        transfer_signed(
            token_program,
            vault1,
            recipient_token1,
            pair,
            excess1,
            pair_signer_seeds,
        )?;
    }

    Ok(())
}
