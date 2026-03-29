use solana_program::{pubkey, pubkey::Pubkey};

/// Seed used to derive the singleton factory PDA.
pub const FACTORY_SEED: &[u8; 7] = b"factory";
/// Prefix used to derive pair PDAs from `(factory, mint0, mint1)`.
pub const PAIR_SEED_PREFIX: &[u8; 4] = b"pair";
/// Prefix used to derive the LP mint PDA for a pair.
pub const LP_MINT_SEED_PREFIX: &[u8; 7] = b"lp_mint";

/// Fixed swap fee charged by the pool, expressed in basis points.
pub const SWAP_FEE_BPS: u16 = 3; // 0.03%
/// Decimal precision used for LP tokens minted by this program.
pub const LP_DECIMALS: u8 = 6;

/// Permanently locked LP supply used to prevent edge cases.
pub const MINIMUM_LIQUIDITY: u64 = 1000;
/// Program admin allowed to initialize the factory, pair and receive protocol fees.
pub const PROGRAM_ADMIN_AUTHORITY: Pubkey = pubkey!("BNToqmqXLNvUrEGGS7io3MQodB9dT56M4Q1Q8xcPYyk7");
