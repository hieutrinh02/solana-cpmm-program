use borsh::{BorshDeserialize, BorshSerialize};
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
use spl_token_interface::{
    self,
    state::{Account as TokenAccount, Mint},
};

use super::lib::{
    assert_token_account, create_ata_if_needed, derive_factory_pda, derive_pair_pda, get_ata,
    mint_fee_if_needed, mint_to_signed, transfer, update_reserves_and_twap, FeeMintAccounts,
    SplProgramAccounts,
};
use crate::{
    constants::{MINIMUM_LIQUIDITY, PAIR_SEED_PREFIX},
    math::{min_u128, quote, sqrt_u128, u128_to_u64},
    state::PairState,
};

/// Adds liquidity into an existing pair or bootstraps the initial reserves.
///
/// On the first deposit the instruction permanently locks
/// `MINIMUM_LIQUIDITY` LP tokens to the pair PDA to avoid edge cases around total supply.
pub fn add_liquidity<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    amount0_desired: u64,
    amount1_desired: u64,
    amount0_min: u64,
    amount1_min: u64,
) -> ProgramResult {
    // Accounts
    // 0) payer (signer, writable)
    // 1) pair_pda (writable) - PairState (program-owned) + authority
    // 2) payer_token0_ata (writable)
    // 3) payer_token1_ata (writable)
    // 4) vault0_ata (writable) - ATA (pair, mint0)
    // 5) vault1_ata (writable) - ATA (pair, mint1)
    // 6) lp_mint_pda (writable)
    // 7) payer_lp_ata (writable) - ATA (payer, lp_mint)
    // 8) locked_lp_ata (writable) - ATA (pair, lp_mint), lock MINIMUM_LIQUIDITY
    // 9) admin (read-only) - protocol fee recipient, must equal PROGRAM_ADMIN_AUTHORITY
    // 10) admin_lp_ata (writable) - ATA(admin, lp_mint)
    // 11) token_program (read-only)
    // 12) ata_program (read-only)
    // 13) system_program (read-only)
    // 14) rent_sysvar (read-only)
    // 15) clock_sysvar (read-only)

    let accounts_iter = &mut accounts.iter();

    let payer = next_account_info(accounts_iter)?;
    let pair = next_account_info(accounts_iter)?;
    let payer_token0 = next_account_info(accounts_iter)?;
    let payer_token1 = next_account_info(accounts_iter)?;
    let vault0 = next_account_info(accounts_iter)?;
    let vault1 = next_account_info(accounts_iter)?;
    let lp_mint = next_account_info(accounts_iter)?;
    let payer_lp = next_account_info(accounts_iter)?;
    let locked_lp = next_account_info(accounts_iter)?;
    let admin = next_account_info(accounts_iter)?;
    let admin_lp = next_account_info(accounts_iter)?;
    let token_program = next_account_info(accounts_iter)?;
    let ata_program = next_account_info(accounts_iter)?;
    let sys_program = next_account_info(accounts_iter)?;
    let rent_sysvar = next_account_info(accounts_iter)?;
    let clock_sysvar = next_account_info(accounts_iter)?;
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

    // Input amount checks.
    // Both desired amounts must be non-zero to create or expand LP shares.
    if amount0_desired == 0 || amount1_desired == 0 {
        msg!("zero desired amount");
        return Err(ProgramError::InvalidArgument);
    }

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
    if *lp_mint.key != pair_state.lp_mint {
        msg!("invalid lp mint");
        return Err(ProgramError::InvalidArgument);
    }

    // Token account and ATA checks.
    assert_token_account(payer_token0, payer.key, &pair_state.token0_mint)?;
    assert_token_account(payer_token1, payer.key, &pair_state.token1_mint)?;
    assert_token_account(vault0, pair.key, &pair_state.token0_mint)?;
    assert_token_account(vault1, pair.key, &pair_state.token1_mint)?;

    let expected_payer_token0 = get_ata(payer.key, &pair_state.token0_mint);
    let expected_payer_token1 = get_ata(payer.key, &pair_state.token1_mint);
    let expected_vault0 = get_ata(pair.key, &pair_state.token0_mint);
    let expected_vault1 = get_ata(pair.key, &pair_state.token1_mint);

    if *payer_token0.key != expected_payer_token0 {
        msg!("invalid payer token0 ata");
        return Err(ProgramError::InvalidArgument);
    }
    if *payer_token1.key != expected_payer_token1 {
        msg!("invalid payer token1 ata");
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

    // LP mint checks.
    // The LP mint is token-program owned and its supply is later used to
    // compute minted shares.
    if lp_mint.owner != token_program.key {
        msg!("lp mint not owned by token program");
        return Err(ProgramError::IllegalOwner);
    }

    // LP token account checks and preparation.
    create_ata_if_needed(payer, payer_lp, payer, lp_mint, &spl_programs)?;
    create_ata_if_needed(payer, locked_lp, pair, lp_mint, &spl_programs)?;
    assert_token_account(payer_lp, payer.key, lp_mint.key)?;
    assert_token_account(locked_lp, pair.key, lp_mint.key)?;

    let expected_payer_lp = get_ata(payer.key, lp_mint.key);
    let expected_locked_lp = get_ata(pair.key, lp_mint.key);
    if *payer_lp.key != expected_payer_lp {
        msg!("invalid payer lp ATA");
        return Err(ProgramError::InvalidArgument);
    }
    if *locked_lp.key != expected_locked_lp {
        msg!("invalid locked lp ATA");
        return Err(ProgramError::InvalidArgument);
    }

    let reserve0 = pair_state.reserve0;
    let reserve1 = pair_state.reserve1;

    // Business logic setup.
    // The pair PDA is the authority for vault withdrawals and LP minting.
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
    let fee_accounts = FeeMintAccounts {
        payer,
        pair,
        lp_mint,
        fee_recipient: admin,
        fee_recipient_lp: admin_lp,
    };

    // Protocol fee handling.
    // Mint any pending protocol fee before computing user shares so the LP
    // supply used below reflects the fee-on path.
    mint_fee_if_needed(
        &mut pair_state,
        &fee_accounts,
        &spl_programs,
        pair_signer_seeds,
    )?;

    let lp_mint_state =
        Mint::unpack(&lp_mint.data.borrow()).map_err(|_| ProgramError::InvalidAccountData)?;
    let total_supply = lp_mint_state.supply;

    // Supply and reserve consistency checks.
    if (reserve0 == 0 || reserve1 == 0) && total_supply != 0 {
        msg!("invalid pool state: zero reserve with non-zero supply");
        return Err(ProgramError::InvalidAccountData);
    }

    let (amount0_used, amount1_used, user_liquidity, lock_minimum) = if reserve0 == 0
        && reserve1 == 0
    {
        // Initial liquidity path.
        if amount0_desired < amount0_min || amount1_desired < amount1_min {
            msg!("initial liquidity below min constraints");
            return Err(ProgramError::InvalidArgument);
        }

        let product = (amount0_desired as u128)
            .checked_mul(amount1_desired as u128)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        let liquidity_raw = sqrt_u128(product);
        let min_liq = MINIMUM_LIQUIDITY as u128;

        if liquidity_raw <= min_liq {
            msg!("insufficient initial liquidity");
            return Err(ProgramError::InvalidArgument);
        }

        let minted_to_user = liquidity_raw
            .checked_sub(min_liq)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        (
            amount0_desired,
            amount1_desired,
            u128_to_u64(minted_to_user)?,
            true,
        )
    } else {
        // Existing liquidity path.
        let reserve0_u128 = reserve0 as u128;
        let reserve1_u128 = reserve1 as u128;
        let amount0_desired_u128 = amount0_desired as u128;
        let amount1_desired_u128 = amount1_desired as u128;

        let amount1_optimal = quote(amount0_desired_u128, reserve0_u128, reserve1_u128)?;
        let (amount0_used_u128, amount1_used_u128) = if amount1_optimal <= amount1_desired_u128 {
            if amount1_optimal < amount1_min as u128 {
                msg!("amount1 below minimum");
                return Err(ProgramError::InvalidArgument);
            }
            (amount0_desired_u128, amount1_optimal)
        } else {
            let amount0_optimal = quote(amount1_desired_u128, reserve1_u128, reserve0_u128)?;
            if amount0_optimal > amount0_desired_u128 {
                msg!("amount0 optimal exceeds desired");
                return Err(ProgramError::InvalidArgument);
            }
            if amount0_optimal < amount0_min as u128 {
                msg!("amount0 below minimum");
                return Err(ProgramError::InvalidArgument);
            }
            (amount0_optimal, amount1_desired_u128)
        };

        let total_supply_u128 = total_supply as u128;

        let liquidity0 = amount0_used_u128
            .checked_mul(total_supply_u128)
            .ok_or(ProgramError::ArithmeticOverflow)?
            .checked_div(reserve0_u128)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        let liquidity1 = amount1_used_u128
            .checked_mul(total_supply_u128)
            .ok_or(ProgramError::ArithmeticOverflow)?
            .checked_div(reserve1_u128)
            .ok_or(ProgramError::ArithmeticOverflow)?;

        let liquidity = min_u128(liquidity0, liquidity1);
        if liquidity == 0 {
            msg!("insufficient liquidity minted");
            return Err(ProgramError::InvalidArgument);
        }

        (
            u128_to_u64(amount0_used_u128)?,
            u128_to_u64(amount1_used_u128)?,
            u128_to_u64(liquidity)?,
            false,
        )
    };

    // Token transfers and LP minting.
    if amount0_used > 0 {
        transfer(token_program, payer_token0, vault0, payer, amount0_used)?;
    }
    if amount1_used > 0 {
        transfer(token_program, payer_token1, vault1, payer, amount1_used)?;
    }

    // Permanently burn-equivalent lock on the first add so total supply can
    // never drop to zero again.
    if lock_minimum {
        mint_to_signed(
            token_program,
            lp_mint,
            locked_lp,
            pair,
            MINIMUM_LIQUIDITY,
            pair_signer_seeds,
        )?;
    }

    // LP minting to user.
    mint_to_signed(
        token_program,
        lp_mint,
        payer_lp,
        pair,
        user_liquidity,
        pair_signer_seeds,
    )?;

    // State writeback.
    let vault0_state = TokenAccount::unpack(&vault0.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let vault1_state = TokenAccount::unpack(&vault1.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;

    let balance0 = vault0_state.amount;
    let balance1 = vault1_state.amount;

    // Synchronize reserves using the actual post-transfer balances.
    update_reserves_and_twap(&mut pair_state, clock_sysvar, balance0, balance1)?;

    // k_last updating.
    pair_state.k_last = (pair_state.reserve0 as u128)
        .checked_mul(pair_state.reserve1 as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    pair_state
        .serialize(&mut &mut pair.data.borrow_mut()[..])
        .map_err(|_| ProgramError::InvalidAccountData)?;

    Ok(())
}
