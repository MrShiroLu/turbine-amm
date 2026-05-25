use anchor_lang::prelude::*;

// one config per x/y pool
#[account]
#[derive(InitSpace)]
pub struct Config {
    pub seed: u64,                 // lets one wallet open many pools
    pub authority: Option<Pubkey>, // can lock the pool; None = immutable
    pub mint_x: Pubkey,
    pub mint_y: Pubkey,
    pub fee: u16,                  // basis points
    pub locked: bool,
    pub config_bump: u8,
    pub lp_bump: u8,
}
