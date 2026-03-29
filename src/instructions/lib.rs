use solana_address::Address;
#[allow(deprecated)]
use solana_program::{
    account_info::AccountInfo,
    entrypoint::ProgramResult,
    instruction::{AccountMeta, Instruction},
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    pubkey::Pubkey,
    system_instruction, system_program,
    sysvar::{
        clock::{id as sysvar_clock_id, Clock},
        Sysvar,
    },
};
use solana_program_pack::Pack;
use spl_associated_token_account_interface as spl_ata;
use spl_token_interface::{
    self,
    state::{Account, Mint},
};

use crate::{
    constants::{
        FACTORY_SEED, LP_MINT_SEED_PREFIX, PAIR_SEED_PREFIX, PROGRAM_ADMIN_AUTHORITY, SWAP_FEE_BPS,
    },
    math::{sqrt_u128, u128_to_u64},
    state::PairState,
};

/// Shared SPL and sysvar accounts passed to CPI helper functions.
pub struct SplProgramAccounts<'a> {
    pub token_program: &'a AccountInfo<'a>,
    pub ata_program: &'a AccountInfo<'a>,
    pub system_program: &'a AccountInfo<'a>,
    pub rent_sysvar: &'a AccountInfo<'a>,
}

/// Accounts required by the protocol-fee minting path.
pub struct FeeMintAccounts<'a> {
    pub payer: &'a AccountInfo<'a>,
    pub pair: &'a AccountInfo<'a>,
    pub lp_mint: &'a AccountInfo<'a>,
    pub fee_recipient: &'a AccountInfo<'a>,
    pub fee_recipient_lp: &'a AccountInfo<'a>,
}

/// Derives the singleton factory PDA.
pub fn derive_factory_pda(program_id: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[FACTORY_SEED], program_id)
}

/// Derives the pair PDA from the factory and canonically sorted mint pair.
pub fn derive_pair_pda(
    program_id: &Pubkey,
    factory: &Pubkey,
    mint0: &Pubkey,
    mint1: &Pubkey,
) -> (Pubkey, u8) {
    Pubkey::find_program_address(
        &[
            PAIR_SEED_PREFIX,
            factory.as_ref(),
            mint0.as_ref(),
            mint1.as_ref(),
        ],
        program_id,
    )
}

/// Derives the LP mint PDA for a given pair PDA.
pub fn derive_lp_mint_pda(program_id: &Pubkey, pair: &Pubkey) -> (Pubkey, u8) {
    Pubkey::find_program_address(&[LP_MINT_SEED_PREFIX, pair.as_ref()], program_id)
}

/// Returns the canonical associated token address for `(owner, mint)`.
pub fn get_ata(owner: &Pubkey, mint: &Pubkey) -> Pubkey {
    let addr = spl_ata::address::get_associated_token_address(
        &Address::from(owner.to_bytes()),
        &Address::from(mint.to_bytes()),
    );
    Pubkey::from(addr.to_bytes())
}

/// Creates an ATA if it does not already exist.
pub fn create_ata_if_needed<'a>(
    payer: &AccountInfo<'a>,
    ata: &AccountInfo<'a>,
    owner: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    programs: &SplProgramAccounts<'a>,
) -> ProgramResult {
    let spl_ix = spl_ata::instruction::create_associated_token_account_idempotent(
        &Address::from(payer.key.to_bytes()),
        &Address::from(owner.key.to_bytes()),
        &Address::from(mint.key.to_bytes()),
        &Address::from(programs.token_program.key.to_bytes()),
    );
    let ix: Instruction = Instruction {
        program_id: Pubkey::from(spl_ix.program_id.to_bytes()),
        accounts: spl_ix
            .accounts
            .iter()
            .map(|acc| AccountMeta {
                pubkey: Pubkey::from(acc.pubkey.to_bytes()),
                is_signer: acc.is_signer,
                is_writable: acc.is_writable,
            })
            .collect(),
        data: spl_ix.data,
    };

    invoke(
        &ix,
        &[
            payer.clone(), // Funding account
            ata.clone(),   // ATA to create
            owner.clone(), // Wallet owner
            mint.clone(),  // Mint
            programs.system_program.clone(),
            programs.token_program.clone(),
            programs.ata_program.clone(),
            programs.rent_sysvar.clone(),
        ],
    )
    .map_err(|_| ProgramError::InvalidInstructionData)
}

/// Creates or claims a PDA-backed system account.
///
/// This helper supports the case where the PDA has already been prefunded with
/// lamports as a system-owned zero-data account. In that scenario, it tops up
/// rent if necessary, then uses `allocate` + `assign` with PDA signing instead
/// of relying on `create_account`, which would otherwise fail.
pub fn create_or_claim_pda_account<'a>(
    payer: &AccountInfo<'a>,
    new_account: &AccountInfo<'a>,
    owner_program: &Pubkey,
    lamports: u64,
    space: u64,
    system_program_ai: &AccountInfo<'a>,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    if new_account.lamports() == 0 {
        let ix = system_instruction::create_account(
            payer.key,
            new_account.key,
            lamports,
            space,
            owner_program,
        );

        return invoke_signed(
            &ix,
            &[
                payer.clone(),
                new_account.clone(),
                system_program_ai.clone(),
            ],
            &[signer_seeds],
        );
    }

    if *new_account.owner != system_program::id() || !new_account.data_is_empty() {
        msg!("account already initialized");
        return Err(ProgramError::AccountAlreadyInitialized);
    }

    let missing_lamports = lamports.saturating_sub(new_account.lamports());
    if missing_lamports > 0 {
        let transfer_ix =
            system_instruction::transfer(payer.key, new_account.key, missing_lamports);
        invoke(
            &transfer_ix,
            &[
                payer.clone(),
                new_account.clone(),
                system_program_ai.clone(),
            ],
        )?;
    }

    let allocate_ix = system_instruction::allocate(new_account.key, space);
    invoke_signed(
        &allocate_ix,
        &[new_account.clone(), system_program_ai.clone()],
        &[signer_seeds],
    )?;

    let assign_ix = system_instruction::assign(new_account.key, owner_program);
    invoke_signed(
        &assign_ix,
        &[new_account.clone(), system_program_ai.clone()],
        &[signer_seeds],
    )
}

/// Creates a PDA-owned SPL mint account.
pub fn create_mint_account_signed<'a>(
    payer: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    token_program: &AccountInfo<'a>,
    system_program: &AccountInfo<'a>,
    lamports: u64,
    space: u64,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    create_or_claim_pda_account(
        payer,
        mint,
        token_program.key,
        lamports,
        space,
        system_program,
        signer_seeds,
    )
}

/// Verifies that a token account matches the expected authority and mint.
pub fn assert_token_account<'a>(
    token_acc: &AccountInfo<'a>,
    expected_authority: &Pubkey,
    expected_mint: &Pubkey,
) -> ProgramResult {
    let acc =
        Account::unpack(&token_acc.data.borrow()).map_err(|_| ProgramError::InvalidAccountData)?;

    let authority = Pubkey::from(acc.owner.to_bytes());
    let mint = Pubkey::from(acc.mint.to_bytes());

    if authority != *expected_authority {
        return Err(ProgramError::IllegalOwner);
    }
    if mint != *expected_mint {
        return Err(ProgramError::InvalidAccountData);
    }
    Ok(())
}

/// Initializes an SPL mint.
pub fn init_mint<'a>(
    token_program: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    mint_authority: &AccountInfo<'a>,
    rent_sysvar: &AccountInfo<'a>,
    decimals: u8,
) -> ProgramResult {
    let spl_ix = spl_token_interface::instruction::initialize_mint(
        &Address::from(token_program.key.to_bytes()),
        &Address::from(mint.key.to_bytes()),
        &Address::from(mint_authority.key.to_bytes()),
        None,
        decimals,
    )
    .map_err(|_| ProgramError::InvalidInstructionData)?;
    let ix = Instruction {
        program_id: Pubkey::from(spl_ix.program_id.to_bytes()),
        accounts: spl_ix
            .accounts
            .iter()
            .map(|acc| AccountMeta {
                pubkey: Pubkey::from(acc.pubkey.to_bytes()),
                is_signer: acc.is_signer,
                is_writable: acc.is_writable,
            })
            .collect(),
        data: spl_ix.data,
    };

    invoke(&ix, &[mint.clone(), rent_sysvar.clone()])
        .map_err(|_| ProgramError::InvalidInstructionData)
}

/// Transfers SPL tokens where the authority is an external signer.
pub fn transfer<'a>(
    token_program: &AccountInfo<'a>,
    src: &AccountInfo<'a>,
    dst: &AccountInfo<'a>,
    // Transfer authority
    auth: &AccountInfo<'a>,
    amount: u64,
) -> ProgramResult {
    let spl_ix = spl_token_interface::instruction::transfer(
        &Address::from(token_program.key.to_bytes()),
        &Address::from(src.key.to_bytes()),
        &Address::from(dst.key.to_bytes()),
        &Address::from(auth.key.to_bytes()),
        // Signer pubkeys
        &[],
        amount,
    )
    .map_err(|_| ProgramError::InvalidInstructionData)?;

    let ix = Instruction {
        program_id: Pubkey::from(spl_ix.program_id.to_bytes()),
        accounts: spl_ix
            .accounts
            .iter()
            .map(|acc| AccountMeta {
                pubkey: Pubkey::from(acc.pubkey.to_bytes()),
                is_signer: acc.is_signer,
                is_writable: acc.is_writable,
            })
            .collect(),
        data: spl_ix.data,
    };

    invoke(
        &ix,
        &[
            src.clone(),
            dst.clone(),
            auth.clone(),
            token_program.clone(),
        ],
    )
}

/// Transfers SPL tokens where the authority is a PDA signing via seeds.
pub fn transfer_signed<'a>(
    token_program: &AccountInfo<'a>,
    src: &AccountInfo<'a>,
    dst: &AccountInfo<'a>,
    // Transfer authority
    auth: &AccountInfo<'a>,
    amount: u64,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    let spl_ix = spl_token_interface::instruction::transfer(
        &Address::from(token_program.key.to_bytes()),
        &Address::from(src.key.to_bytes()),
        &Address::from(dst.key.to_bytes()),
        &Address::from(auth.key.to_bytes()),
        // Signer pubkeys
        &[],
        amount,
    )
    .map_err(|_| ProgramError::InvalidInstructionData)?;

    let ix = Instruction {
        program_id: Pubkey::from(spl_ix.program_id.to_bytes()),
        accounts: spl_ix
            .accounts
            .iter()
            .map(|acc| AccountMeta {
                pubkey: Pubkey::from(acc.pubkey.to_bytes()),
                is_signer: acc.is_signer,
                is_writable: acc.is_writable,
            })
            .collect(),
        data: spl_ix.data,
    };

    invoke_signed(
        &ix,
        &[
            src.clone(),
            dst.clone(),
            auth.clone(),
            token_program.clone(),
        ],
        &[signer_seeds],
    )
}

/// Mints LP tokens from a mint controlled by a PDA authority.
pub fn mint_to_signed<'a>(
    token_program: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    to: &AccountInfo<'a>,
    // Mint authority
    auth: &AccountInfo<'a>,
    amount: u64,
    signer_seeds: &[&[u8]],
) -> ProgramResult {
    let spl_ix = spl_token_interface::instruction::mint_to(
        &Address::from(token_program.key.to_bytes()),
        &Address::from(mint.key.to_bytes()),
        &Address::from(to.key.to_bytes()),
        &Address::from(auth.key.to_bytes()),
        &[],
        amount,
    )
    .map_err(|_| ProgramError::InvalidInstructionData)?;

    let ix = Instruction {
        program_id: Pubkey::from(spl_ix.program_id.to_bytes()),
        accounts: spl_ix
            .accounts
            .iter()
            .map(|acc| AccountMeta {
                pubkey: Pubkey::from(acc.pubkey.to_bytes()),
                is_signer: acc.is_signer,
                is_writable: acc.is_writable,
            })
            .collect(),
        data: spl_ix.data,
    };

    invoke_signed(
        &ix,
        &[
            mint.clone(),
            to.clone(),
            auth.clone(),
            token_program.clone(),
        ],
        &[signer_seeds],
    )
}

/// Burns SPL tokens from a token account owned by an external signer.
pub fn burn<'a>(
    token_program: &AccountInfo<'a>,
    from: &AccountInfo<'a>,
    mint: &AccountInfo<'a>,
    auth: &AccountInfo<'a>,
    amount: u64,
) -> ProgramResult {
    let spl_ix = spl_token_interface::instruction::burn(
        &Address::from(token_program.key.to_bytes()),
        &Address::from(from.key.to_bytes()),
        &Address::from(mint.key.to_bytes()),
        &Address::from(auth.key.to_bytes()),
        &[],
        amount,
    )
    .map_err(|_| ProgramError::InvalidInstructionData)?;

    let ix = Instruction {
        program_id: Pubkey::from(spl_ix.program_id.to_bytes()),
        accounts: spl_ix
            .accounts
            .iter()
            .map(|acc| AccountMeta {
                pubkey: Pubkey::from(acc.pubkey.to_bytes()),
                is_signer: acc.is_signer,
                is_writable: acc.is_writable,
            })
            .collect(),
        data: spl_ix.data,
    };

    invoke(
        &ix,
        &[
            from.clone(),
            mint.clone(),
            auth.clone(),
            token_program.clone(),
        ],
    )
}

/// Mints protocol fee LP tokens to the factory admin when growth since
/// `k_last` warrants it.
///
/// This captures one-sixth of pool growth measured as the increase in `sqrt(k)`.
pub fn mint_fee_if_needed<'a>(
    pair_state: &mut PairState,
    fee_accounts: &FeeMintAccounts<'a>,
    programs: &SplProgramAccounts<'a>,
    pair_signer_seeds: &[&[u8]],
) -> ProgramResult {
    if *fee_accounts.fee_recipient.key != PROGRAM_ADMIN_AUTHORITY {
        msg!("invalid fee recipient account");
        return Err(ProgramError::InvalidArgument);
    }

    if pair_state.k_last == 0 {
        return Ok(());
    }

    let current_k = (pair_state.reserve0 as u128)
        .checked_mul(pair_state.reserve1 as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let root_k = sqrt_u128(current_k);
    let root_k_last = sqrt_u128(pair_state.k_last);

    if root_k <= root_k_last {
        return Ok(());
    }

    let lp_mint_state = Mint::unpack(&fee_accounts.lp_mint.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let total_supply = lp_mint_state.supply as u128;

    let numerator = total_supply
        .checked_mul(
            root_k
                .checked_sub(root_k_last)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        )
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let denominator = root_k
        .checked_mul(5)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_add(root_k_last)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let liquidity = numerator
        .checked_div(denominator)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    if liquidity == 0 {
        return Ok(());
    }

    create_ata_if_needed(
        fee_accounts.payer,
        fee_accounts.fee_recipient_lp,
        fee_accounts.fee_recipient,
        fee_accounts.lp_mint,
        programs,
    )?;

    let expected_fee_recipient_lp =
        get_ata(fee_accounts.fee_recipient.key, fee_accounts.lp_mint.key);
    if *fee_accounts.fee_recipient_lp.key != expected_fee_recipient_lp {
        msg!("invalid fee recipient LP ATA");
        return Err(ProgramError::InvalidArgument);
    }

    assert_token_account(
        fee_accounts.fee_recipient_lp,
        fee_accounts.fee_recipient.key,
        fee_accounts.lp_mint.key,
    )?;

    mint_to_signed(
        programs.token_program,
        fee_accounts.lp_mint,
        fee_accounts.fee_recipient_lp,
        fee_accounts.pair,
        u128_to_u64(liquidity)?,
        pair_signer_seeds,
    )?;

    Ok(())
}

/// Updates pair reserves and accumulates TWAP observations from prior reserves.
pub fn update_reserves_and_twap<'a>(
    pair_state: &mut PairState,
    clock_sysvar: &AccountInfo<'a>,
    balance0: u64,
    balance1: u64,
) -> ProgramResult {
    // Verify clock sysvar
    if *clock_sysvar.key != sysvar_clock_id() {
        msg!("invalid clock sysvar");
        return Err(ProgramError::InvalidAccountData);
    }

    let clock =
        Clock::from_account_info(clock_sysvar).map_err(|_| ProgramError::InvalidAccountData)?;

    let current_timestamp =
        u64::try_from(clock.unix_timestamp).map_err(|_| ProgramError::InvalidAccountData)?;

    let reserve0 = pair_state.reserve0;
    let reserve1 = pair_state.reserve1;
    let last_timestamp = pair_state.block_timestamp_last;

    // First touch: initialize timestamp only
    if last_timestamp == 0 {
        pair_state.reserve0 = balance0;
        pair_state.reserve1 = balance1;
        pair_state.block_timestamp_last = current_timestamp;
        return Ok(());
    }

    let time_elapsed = current_timestamp
        .checked_sub(last_timestamp)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    // Accumulate price using old reserves over elapsed time
    if time_elapsed > 0 && reserve0 > 0 && reserve1 > 0 {
        // Q64.64 fixed-point spot prices
        let price0_x64 = (reserve1 as u128)
            .checked_shl(64)
            .ok_or(ProgramError::ArithmeticOverflow)?
            .checked_div(reserve0 as u128)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        let price1_x64 = (reserve0 as u128)
            .checked_shl(64)
            .ok_or(ProgramError::ArithmeticOverflow)?
            .checked_div(reserve1 as u128)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        pair_state.price0_cumulative_last = pair_state
            .price0_cumulative_last
            .checked_add(
                price0_x64
                    .checked_mul(time_elapsed as u128)
                    .ok_or(ProgramError::ArithmeticOverflow)?,
            )
            .ok_or(ProgramError::ArithmeticOverflow)?;

        pair_state.price1_cumulative_last = pair_state
            .price1_cumulative_last
            .checked_add(
                price1_x64
                    .checked_mul(time_elapsed as u128)
                    .ok_or(ProgramError::ArithmeticOverflow)?,
            )
            .ok_or(ProgramError::ArithmeticOverflow)?;
    }

    // Set new reserves and timestamp
    pair_state.reserve0 = balance0;
    pair_state.reserve1 = balance1;
    pair_state.block_timestamp_last = current_timestamp;

    Ok(())
}

/// Returns the fixed swap fee used by all pools, in basis points.
pub fn swap_fee_bps() -> u128 {
    SWAP_FEE_BPS as u128
}
