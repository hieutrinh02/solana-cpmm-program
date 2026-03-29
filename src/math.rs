use solana_program::program_error::ProgramError;

/// Returns the smaller of two `u128` values.
pub fn min_u128(a: u128, b: u128) -> u128 {
    a.min(b)
}

/// Converts `u128` to `u64`, returning overflow if the value does not fit.
pub fn u128_to_u64(x: u128) -> Result<u64, ProgramError> {
    if x > u64::MAX as u128 {
        return Err(ProgramError::ArithmeticOverflow);
    }
    Ok(x as u64)
}

/// Computes the integer square root using the Babylonian method.
pub fn sqrt_u128(y: u128) -> u128 {
    if y > 3 {
        let mut z = y;
        let mut x = y / 2 + 1;

        while x < z {
            z = x;
            x = (y / x + x) / 2;
        }

        z
    } else if y != 0 {
        1
    } else {
        0
    }
}

/// Quotes the proportional amount of token B for a given amount of token A.
///
/// Returns `InvalidArgument` when any input is zero and arithmetic overflow when
/// intermediate multiplication or division fails.
pub fn quote(amount_a: u128, reserve_a: u128, reserve_b: u128) -> Result<u128, ProgramError> {
    if amount_a == 0 || reserve_a == 0 || reserve_b == 0 {
        return Err(ProgramError::InvalidArgument);
    }

    amount_a
        .checked_mul(reserve_b)
        .ok_or(ProgramError::ArithmeticOverflow)?
        .checked_div(reserve_a)
        .ok_or(ProgramError::ArithmeticOverflow)
}
