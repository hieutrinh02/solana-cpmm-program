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
    assert_token_account, burn, create_ata_if_needed, derive_factory_pda, derive_pair_pda, get_ata,
    mint_fee_if_needed, transfer_signed, update_reserves_and_twap, FeeMintAccounts,
    SplProgramAccounts,
};
use crate::{constants::PAIR_SEED_PREFIX, math::u128_to_u64, state::PairState};

/// Burns LP tokens and returns the proportional share of underlying reserves.
///
/// Protocol fees are minted first so the burn amount is valued against the
/// latest effective LP supply.
pub fn remove_liquidity<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    liquidity: u64,
    amount0_min: u64,
    amount1_min: u64,
) -> ProgramResult {
    // Accounts
    // 0) payer (signer, writable)
    // 1) pair_pda (writable) - PairState (program-owned) + authority
    // 2) token0_mint (read-only)
    // 3) token1_mint (read-only)
    // 4) payer_token0_ata (writable)
    // 5) payer_token1_ata (writable)
    // 6) vault0_ata (writable) - ATA(pair, mint0)
    // 7) vault1_ata (writable) - ATA(pair, mint1)
    // 8) lp_mint_pda (writable)
    // 9) payer_lp_ata (writable) - ATA(payer, lp_mint)
    // 10) admin (read-only) - protocol fee recipient, must equal PROGRAM_ADMIN_AUTHORITY
    // 11) admin_lp_ata (writable) - ATA(admin, lp_mint)
    // 12) token_program (read-only)
    // 13) ata_program (read-only)
    // 14) system_program (read-only)
    // 15) rent_sysvar (read-only)
    // 16) clock_sysvar (read-only)

    let accounts_iter = &mut accounts.iter();

    let payer = next_account_info(accounts_iter)?;
    let pair = next_account_info(accounts_iter)?;
    let token0_mint = next_account_info(accounts_iter)?;
    let token1_mint = next_account_info(accounts_iter)?;
    let payer_token0 = next_account_info(accounts_iter)?;
    let payer_token1 = next_account_info(accounts_iter)?;
    let vault0 = next_account_info(accounts_iter)?;
    let vault1 = next_account_info(accounts_iter)?;
    let lp_mint = next_account_info(accounts_iter)?;
    let payer_lp = next_account_info(accounts_iter)?;
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
    // A zero burn would produce zero output and is always invalid.
    if liquidity == 0 {
        msg!("insufficient liquidity burned");
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
    if *token0_mint.key != pair_state.token0_mint {
        msg!("invalid token0 mint");
        return Err(ProgramError::InvalidArgument);
    }
    if *token1_mint.key != pair_state.token1_mint {
        msg!("invalid token1 mint");
        return Err(ProgramError::InvalidArgument);
    }

    let expected_payer_token0 = get_ata(payer.key, &pair_state.token0_mint);
    let expected_payer_token1 = get_ata(payer.key, &pair_state.token1_mint);
    let expected_vault0 = get_ata(pair.key, &pair_state.token0_mint);
    let expected_vault1 = get_ata(pair.key, &pair_state.token1_mint);
    let expected_payer_lp = get_ata(payer.key, lp_mint.key);

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
    if *payer_lp.key != expected_payer_lp {
        msg!("invalid payer lp ATA");
        return Err(ProgramError::InvalidArgument);
    }

    create_ata_if_needed(payer, payer_token0, payer, token0_mint, &spl_programs)?;
    create_ata_if_needed(payer, payer_token1, payer, token1_mint, &spl_programs)?;

    // Token account and ATA checks.
    assert_token_account(payer_token0, payer.key, &pair_state.token0_mint)?;
    assert_token_account(payer_token1, payer.key, &pair_state.token1_mint)?;
    assert_token_account(vault0, pair.key, &pair_state.token0_mint)?;
    assert_token_account(vault1, pair.key, &pair_state.token1_mint)?;
    assert_token_account(payer_lp, payer.key, lp_mint.key)?;

    // LP mint checks.
    if lp_mint.owner != token_program.key {
        msg!("lp mint not owned by token program");
        return Err(ProgramError::IllegalOwner);
    }

    // Business logic setup.
    // The pair PDA signs vault transfers back to the liquidity provider.
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
    // Fee logic must run before redeeming LP shares.
    mint_fee_if_needed(
        &mut pair_state,
        &fee_accounts,
        &spl_programs,
        pair_signer_seeds,
    )?;

    // Balance checks.
    // Use actual vault balances after fee minting to preserve pro-rata redemption against the live pool state.
    let vault0_state = TokenAccount::unpack(&vault0.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;
    let vault1_state = TokenAccount::unpack(&vault1.data.borrow())
        .map_err(|_| ProgramError::InvalidAccountData)?;

    let balance0 = vault0_state.amount;
    let balance1 = vault1_state.amount;

    if balance0 == 0 || balance1 == 0 {
        msg!("insufficient liquidity");
        return Err(ProgramError::InvalidAccountData);
    }

    // Supply checks.
    let lp_mint_state =
        Mint::unpack(&lp_mint.data.borrow()).map_err(|_| ProgramError::InvalidAccountData)?;
    let total_supply = lp_mint_state.supply;

    if total_supply == 0 {
        msg!("insufficient lp supply");
        return Err(ProgramError::InvalidAccountData);
    }
    if liquidity >= total_supply {
        msg!("cannot burn entire supply");
        return Err(ProgramError::InvalidArgument);
    }

    let amount0 = (liquidity as u128)
        .checked_mul(balance0 as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_div(total_supply as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;
    let amount1 = (liquidity as u128)
        .checked_mul(balance1 as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_div(total_supply as u128)
        .ok_or(ProgramError::ArithmeticOverflow)?;

    let amount0 = u128_to_u64(amount0)?;
    let amount1 = u128_to_u64(amount1)?;

    if amount0 == 0 || amount1 == 0 {
        msg!("insufficient output amount");
        return Err(ProgramError::InvalidArgument);
    }
    if amount0 < amount0_min || amount1 < amount1_min {
        msg!("withdrawal amount below min constraints");
        return Err(ProgramError::InvalidArgument);
    }

    // LP burn and token transfers.
    burn(token_program, payer_lp, lp_mint, payer, liquidity)?;

    transfer_signed(
        token_program,
        vault0,
        payer_token0,
        pair,
        amount0,
        pair_signer_seeds,
    )?;
    transfer_signed(
        token_program,
        vault1,
        payer_token1,
        pair,
        amount1,
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
