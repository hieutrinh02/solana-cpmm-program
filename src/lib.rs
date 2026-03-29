use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, pubkey::Pubkey,
};

/// Program-wide constants.
pub mod constants;
/// Instruction handlers and CPI helpers.
pub mod instructions;
/// Arithmetic helpers for AMM math.
pub mod math;
/// On-chain account state definitions.
pub mod state;

/// Entry-point instruction enum decoded from instruction data.
#[derive(BorshDeserialize, BorshSerialize)]
pub enum Cmd {
    InitFactory,
    CreatePair,
    AddLiquidity {
        amount0_desired: u64,
        amount1_desired: u64,
        amount0_min: u64,
        amount1_min: u64,
    },
    Swap {
        amount0_out: u64,
        amount1_out: u64,
    },
    RemoveLiquidity {
        liquidity: u64,
        amount0_min: u64,
        amount1_min: u64,
    },
    Skim,
    Sync,
}

entrypoint!(process_instruction);

/// Dispatches incoming instructions to the corresponding handler.
pub fn process_instruction<'a>(
    program_id: &Pubkey,
    accounts: &'a [AccountInfo<'a>],
    instruction_data: &[u8],
) -> ProgramResult {
    let ix = Cmd::try_from_slice(instruction_data)?;

    match ix {
        Cmd::InitFactory => {
            instructions::init_factory(program_id, accounts)?;
        }
        Cmd::CreatePair => {
            instructions::create_pair(program_id, accounts)?;
        }
        Cmd::AddLiquidity {
            amount0_desired,
            amount1_desired,
            amount0_min,
            amount1_min,
        } => {
            instructions::add_liquidity(
                program_id,
                accounts,
                amount0_desired,
                amount1_desired,
                amount0_min,
                amount1_min,
            )?;
        }
        Cmd::Swap {
            amount0_out,
            amount1_out,
        } => {
            instructions::swap(program_id, accounts, amount0_out, amount1_out)?;
        }
        Cmd::RemoveLiquidity {
            liquidity,
            amount0_min,
            amount1_min,
        } => {
            instructions::remove_liquidity(
                program_id,
                accounts,
                liquidity,
                amount0_min,
                amount1_min,
            )?;
        }
        Cmd::Skim => {
            instructions::skim(program_id, accounts)?;
        }
        Cmd::Sync => {
            instructions::sync(program_id, accounts)?;
        }
    }

    Ok(())
}
