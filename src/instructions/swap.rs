use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program_error::ProgramError,
    pubkey::Pubkey,
};
use solana_program_pack::Pack;
use spl_token_interface::{self, state::Account as TokenAccount};

use super::lib::{
    assert_token_account, derive_factory_pda, derive_pair_pda, get_ata, swap_fee_bps,
    transfer_signed, update_reserves_and_twap,
};
use crate::{constants::PAIR_SEED_PREFIX, state::PairState};

/// Executes a swap by sending tokens out first and inferring tokens in from
/// the resulting vault balances.
///
/// This mirrors the classic constant-product flow where the caller can
/// transfer input tokens before or during the same transaction and the program
/// validates the final invariant from observed balances.
pub fn swap<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    amount0_out: u64,
    amount1_out: u64,
) -> ProgramResult {
    // Accounts
    // 0) user (signer, writable)
    // 1) pair_pda (writable) - PairState
    // 2) user_token0_ata (writable)
    // 3) user_token1_ata (writable)
    // 4) vault0_ata (writable)
    // 5) vault1_ata (writable)
    // 6) token_program (read-only)
    // 7) clock_sysvar (read-only)

    let accounts_iter = &mut accounts.iter();

    let user = next_account_info(accounts_iter)?;
    let pair = next_account_info(accounts_iter)?;
    let user_token0 = next_account_info(accounts_iter)?;
    let user_token1 = next_account_info(accounts_iter)?;
    let vault0 = next_account_info(accounts_iter)?;
    let vault1 = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;

    // Authority checks.
    if !user.is_signer {
        msg!("user must be a signer");
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Static program checks.
    if token_program.key.to_bytes() != spl_token_interface::id().to_bytes() {
        msg!("invalid token program");
        return Err(ProgramError::IncorrectProgramId);
    }

    // Input amount checks.
    if amount0_out == 0 && amount1_out == 0 {
        msg!("insufficient output amount");
        return Err(ProgramError::InvalidArgument);
    }

    // PDA ownership checks.
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

    // Token account and ATA checks.
    assert_token_account(user_token0, user.key, &pair_state.token0_mint)?;
    assert_token_account(user_token1, user.key, &pair_state.token1_mint)?;
    assert_token_account(vault0, pair.key, &pair_state.token0_mint)?;
    assert_token_account(vault1, pair.key, &pair_state.token1_mint)?;

    let expected_user_token0 = get_ata(user.key, &pair_state.token0_mint);
    let expected_user_token1 = get_ata(user.key, &pair_state.token1_mint);
    let expected_vault0 = get_ata(pair.key, &pair_state.token0_mint);
    let expected_vault1 = get_ata(pair.key, &pair_state.token1_mint);

    if *user_token0.key != expected_user_token0 {
        msg!("invalid user token0 ata");
        return Err(ProgramError::InvalidArgument);
    }
    if *user_token1.key != expected_user_token1 {
        msg!("invalid user token1 ata");
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

    let reserve0 = pair_state.reserve0;
    let reserve1 = pair_state.reserve1;

    // Reserve checks.
    if reserve0 == 0 || reserve1 == 0 {
        msg!("insufficient liquidity");
        return Err(ProgramError::InvalidAccountData);
    }

    // Output reserve bounds.
    if amount0_out >= reserve0 || amount1_out >= reserve1 {
        msg!("insufficient liquidity");
        return Err(ProgramError::InvalidArgument);
    }

    // Business logic setup.
    // The pair PDA is the authority over both vault ATAs.
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

    // Transfers.
    if amount0_out > 0 {
        transfer_signed(
            token_program,
            vault0,
            user_token0,
            pair,
            amount0_out,
            pair_signer_seeds,
        )?;
    }
    if amount1_out > 0 {
        transfer_signed(
            token_program,
            vault1,
            user_token1,
            pair,
            amount1_out,
            pair_signer_seeds,
        )?;
    }

    // Balance observation.
    let vault0_state = TokenAccount::unpack(&vault0.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let vault1_state = TokenAccount::unpack(&vault1.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;

    let balance0 = vault0_state.amount;
    let balance1 = vault1_state.amount;

    // Input inference.
    let reserve0_after_out = reserve0
        .checked_sub(amount0_out)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let reserve1_after_out = reserve1
        .checked_sub(amount1_out)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let amount0_in = if balance0 > reserve0_after_out {
        balance0
            .checked_sub(reserve0_after_out)
            .ok_or(ProgramError::ArithmeticOverflow)?
    } else {
        0
    };
    let amount1_in = if balance1 > reserve1_after_out {
        balance1
            .checked_sub(reserve1_after_out)
            .ok_or(ProgramError::ArithmeticOverflow)?
    } else {
        0
    };

    if amount0_in == 0 && amount1_in == 0 {
        msg!("insufficient input amount");
        return Err(ProgramError::InvalidArgument);
    }

    // Invariant checks.
    // Apply swap fee only to the inferred input amount before checking the
    // constant-product invariant.
    let fee_bps = swap_fee_bps();
    let fee_denominator = 10_000u128;

    let balance0_adjusted = (balance0 as u128)
        .checked_mul(fee_denominator)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_sub(
            (amount0_in as u128)
                .checked_mul(fee_bps)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        )
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let balance1_adjusted = (balance1 as u128)
        .checked_mul(fee_denominator)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_sub(
            (amount1_in as u128)
                .checked_mul(fee_bps)
                .ok_or(ProgramError::ArithmeticOverflow)?,
        )
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let lhs = balance0_adjusted
        .checked_mul(balance1_adjusted)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let rhs = (reserve0 as u128)
        .checked_mul(reserve1 as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_mul(fee_denominator)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_mul(fee_denominator)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    if lhs < rhs {
        msg!("K invariant violation");
        return Err(ProgramError::InvalidArgument);
    }

    // State writeback.
    update_reserves_and_twap(&mut pair_state, clock_sysvar, balance0, balance1)?;

    pair_state
        .serialize(&mut &mut pair.data.borrow_mut()[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;

    Ok(())
}
