use anchor_lang::prelude::*;

#[error_code]
pub enum AmmError {
    #[msg("Fee in basis points cannot exceed 10000 (100%)")]
    InvalidFee,
    #[msg("The two mints of a pool must be different")]
    IdenticalMints,
    #[msg("The pool is locked")]
    PoolLocked,
    #[msg("Amount must be greater than zero")]
    ZeroAmount,
    #[msg("Pool has no liquidity")]
    NoLiquidity,
    #[msg("Slippage tolerance exceeded")]
    SlippageExceeded,
    #[msg("Arithmetic overflow")]
    Overflow,
    #[msg("Computed output is invalid")]
    InvalidOutput,
}
