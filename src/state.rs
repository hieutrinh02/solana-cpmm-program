use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

/// Global configuration for the AMM factory.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone, PartialEq, Eq)]
pub struct FactoryState {
    /// Whether the account has been initialized.
    pub is_initialized: bool,
    /// Number of pairs created by this factory.
    pub pair_count: u64,
}

/// On-chain state for a single constant-product pair.
#[derive(BorshDeserialize, BorshSerialize, Debug, Clone, PartialEq, Eq)]
pub struct PairState {
    /// Whether the pair account has been initialized.
    pub is_initialized: bool,
    /// Factory that owns and created this pair.
    pub factory: Pubkey,

    /// Canonically sorted first token mint.
    pub token0_mint: Pubkey,
    /// Canonically sorted second token mint.
    pub token1_mint: Pubkey,

    /// Vault ATA owned by the pair PDA for `token0_mint`.
    pub vault0: Pubkey,
    /// Vault ATA owned by the pair PDA for `token1_mint`.
    pub vault1: Pubkey,
    /// LP mint PDA for this pair.
    pub lp_mint: Pubkey,

    /// Last synchronized reserve for token0.
    pub reserve0: u64,
    /// Last synchronized reserve for token1.
    pub reserve1: u64,
    /// Last reserve product used for protocol fee minting.
    pub k_last: u128,

    // Cumulative spot prices in Q64.64 format for TWAP calculations.
    pub price0_cumulative_last: u128,
    pub price1_cumulative_last: u128,
    /// Unix timestamp of the last reserve update.
    pub block_timestamp_last: u64,
}

impl FactoryState {
    /// Serialized size of [`FactoryState`].
    pub const LEN: usize = 1 // is_initialized
        + 8; // pair_count
}

impl PairState {
    /// Serialized size of [`PairState`].
    pub const LEN: usize = 1 // is_initialized
        + 32 // factory
        + 32 // token0_mint
        + 32 // token1_mint
        + 32 // vault0
        + 32 // vault1
        + 32 // lp_mint
        + 8 // reserve0
        + 8 // reserve1
        + 16 // k_last
        + 16 // price0_cumulative_last
        + 16 // price1_cumulative_last
        + 8; // block_timestamp_last
}
