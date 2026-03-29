use borsh::{BorshDeserialize, BorshSerialize};
#[allow(deprecated)]
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
    rent::Rent,
    system_program,
    sysvar::{self, Sysvar},
};
use solana_program_pack::Pack;
use spl_associated_token_account_interface;
use spl_token_interface::{self, state::Mint};

use super::lib::{
    assert_token_account, create_ata_if_needed, create_mint_account_signed,
    create_or_claim_pda_account, derive_factory_pda, derive_lp_mint_pda, derive_pair_pda, get_ata,
    init_mint, SplProgramAccounts,
};
use crate::{
    constants::{LP_DECIMALS, LP_MINT_SEED_PREFIX, PAIR_SEED_PREFIX, PROGRAM_ADMIN_AUTHORITY},
    state::{FactoryState, PairState},
};

/// Creates a new pair account, its vault ATAs, and the LP mint PDA.
///
/// The two input mints are canonically sorted before deriving the pair PDA so
/// that `(A, B)` and `(B, A)` always map to the same pool.
pub fn create_pair<'a>(program_id: &Pubkey, accounts: &'a [AccountInfo<'a>]) -> ProgramResult {
    // Accounts
    // 0) payer (signer, writable)
    // 1) factory_pda (writable) - FactoryState (program-owned)
    // 2) pair_pda (writable) - PairState (program-owned) + authority
    // 3) mint_a (read-only)
    // 4) mint_b (read-only)
    // 5) vault_a_ata (writable) - ATA(pair_pda, mint0)
    // 6) vault_b_ata (writable) - ATA(pair_pda, mint1)
    // 7) lp_mint_pda (writable) - mint (owner = token_program)
    // 8) token_program (read-only)
    // 9) ata_program (read-only)
    // 10) system_program (read-only)
    // 11) rent_sysvar (read-only)
    let accounts_iter = &mut accounts.iter();

    let payer = next_account_info(accounts_iter)?;
    let factory = next_account_info(accounts_iter)?;
    let pair = next_account_info(accounts_iter)?;
    let mint_a = next_account_info(accounts_iter)?;
    let mint_b = next_account_info(accounts_iter)?;
    let vault_a = next_account_info(accounts_iter)?;
    let vault_b = next_account_info(accounts_iter)?;
    let lp_mint = next_account_info(accounts_iter)?;
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

    // Authority checks.
    if !payer.is_signer {
        msg!("payer must be a signer");
        return Err(ProgramError::MissingRequiredSignature);
    }
    if *payer.key != PROGRAM_ADMIN_AUTHORITY {
        msg!("payer is not authorized to create pairs");
        return Err(ProgramError::IllegalOwner);
    }

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
        msg!("invalid ATA program");
        return Err(ProgramError::IncorrectProgramId);
    }

    if *rent_sysvar.key != sysvar::rent::id() {
        msg!("invalid rent sysvar");
        return Err(ProgramError::InvalidAccountData);
    }

    // Pair input checks.
    if mint_a.key == mint_b.key {
        msg!("identical mints not allowed");
        return Err(ProgramError::InvalidArgument);
    }

    // PDA ownership checks.
    let (expected_factory, _) = derive_factory_pda(program_id);
    if *factory.key != expected_factory {
        msg!("invalid factory PDA");
        return Err(ProgramError::InvalidSeeds);
    }
    if factory.owner != program_id {
        msg!("factory owner mismatch");
        return Err(ProgramError::IllegalOwner);
    }

    // Factory state checks.
    let mut factory_state = FactoryState::try_from_slice(&factory.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;
    if !factory_state.is_initialized {
        msg!("factory not initialized");
        return Err(ProgramError::UninitializedAccount);
    }

    // Mint account checks.
    if mint_a.owner != token_program.key {
        msg!("mint_a not owned by token program");
        return Err(ProgramError::IllegalOwner);
    }
    if mint_b.owner != token_program.key {
        msg!("mint_b not owned by token program");
        return Err(ProgramError::IllegalOwner);
    }

    Mint::unpack(&mint_a.data.borrow()).map_err(|_| ProgramError::InvalidAccountData)?;
    Mint::unpack(&mint_b.data.borrow()).map_err(|_| ProgramError::InvalidAccountData)?;

    // Canonical ordering.
    let (mint0_ai, mint1_ai) = if mint_a.key.to_bytes() < mint_b.key.to_bytes() {
        (mint_a, mint_b)
    } else {
        (mint_b, mint_a)
    };

    // PDA derivation and existence checks.
    let (expected_pair, pair_bump) =
        derive_pair_pda(program_id, &expected_factory, mint0_ai.key, mint1_ai.key);
    if *pair.key != expected_pair {
        msg!("invalid pair pda");
        return Err(ProgramError::InvalidSeeds);
    }

    // Canonical ATA derivation checks.
    let expected_vault0 = get_ata(&expected_pair, mint0_ai.key);
    let expected_vault1 = get_ata(&expected_pair, mint1_ai.key);

    let (vault0_ai, vault1_ai) =
        if *vault_a.key == expected_vault0 && *vault_b.key == expected_vault1 {
            (vault_a, vault_b)
        } else if *vault_a.key == expected_vault1 && *vault_b.key == expected_vault0 {
            (vault_b, vault_a)
        } else {
            msg!("invalid vault ATAs");
            return Err(ProgramError::InvalidArgument);
        };

    let (expected_lp_mint, lp_mint_bump) = derive_lp_mint_pda(program_id, &expected_pair);
    if *lp_mint.key != expected_lp_mint {
        msg!("invalid lp mint pda");
        return Err(ProgramError::InvalidSeeds);
    }

    // Account creation.
    let rent = Rent::get()?;
    let pair_space: usize = PairState::LEN;
    let pair_lamports = rent.minimum_balance(pair_space);

    create_or_claim_pda_account(
        payer,
        pair,
        program_id,
        pair_lamports,
        pair_space as u64,
        sys_program,
        &[
            PAIR_SEED_PREFIX,
            factory.key.as_ref(),
            mint0_ai.key.as_ref(),
            mint1_ai.key.as_ref(),
            &[pair_bump],
        ],
    )?;

    // ATA creation and validation.
    create_ata_if_needed(payer, vault0_ai, pair, mint0_ai, &spl_programs)?;
    create_ata_if_needed(payer, vault1_ai, pair, mint1_ai, &spl_programs)?;

    assert_token_account(vault0_ai, &expected_pair, mint0_ai.key)?;
    assert_token_account(vault1_ai, &expected_pair, mint1_ai.key)?;

    // LP mint creation.
    let lp_mint_len = Mint::LEN;
    let lp_mint_lamports = rent.minimum_balance(lp_mint_len);

    create_mint_account_signed(
        payer,
        lp_mint,
        token_program,
        sys_program,
        lp_mint_lamports,
        lp_mint_len as u64,
        &[LP_MINT_SEED_PREFIX, expected_pair.as_ref(), &[lp_mint_bump]],
    )?;

    init_mint(token_program, lp_mint, pair, rent_sysvar, LP_DECIMALS)?;

    // State writeback.
    let pair_state = PairState {
        is_initialized: true,
        factory: *factory.key,

        token0_mint: *mint0_ai.key,
        token1_mint: *mint1_ai.key,

        vault0: *vault0_ai.key,
        vault1: *vault1_ai.key,
        lp_mint: *lp_mint.key,

        reserve0: 0,
        reserve1: 0,
        k_last: 0,

        price0_cumulative_last: 0,
        price1_cumulative_last: 0,
        block_timestamp_last: 0,
    };

    pair_state
        .serialize(&mut &mut pair.data.borrow_mut()[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;

    factory_state.pair_count = factory_state
        .pair_count
        .checked_add(1)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    factory_state
        .serialize(&mut &mut factory.data.borrow_mut()[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;

    Ok(())
}
