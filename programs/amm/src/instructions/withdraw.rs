use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Burn, Mint, Token, TokenAccount, Transfer},
};

use crate::{constants::*, error::AmmError, state::Config};

// burn LP, return the proportional share. min_x/min_y are slippage floors
#[derive(Accounts)]
pub struct Withdraw<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(
        seeds = [CONFIG_SEED, config.seed.to_le_bytes().as_ref()],
        bump = config.config_bump,
        has_one = mint_x,
        has_one = mint_y,
    )]
    pub config: Box<Account<'info, Config>>,

    pub mint_x: Box<Account<'info, Mint>>,
    pub mint_y: Box<Account<'info, Mint>>,

    #[account(
        mut,
        seeds = [LP_SEED, config.key().as_ref()],
        bump = config.lp_bump,
    )]
    pub mint_lp: Box<Account<'info, Mint>>,

    #[account(mut, associated_token::mint = mint_x, associated_token::authority = config)]
    pub vault_x: Box<Account<'info, TokenAccount>>,
    #[account(mut, associated_token::mint = mint_y, associated_token::authority = config)]
    pub vault_y: Box<Account<'info, TokenAccount>>,

    #[account(mut, associated_token::mint = mint_x, associated_token::authority = user)]
    pub user_x: Box<Account<'info, TokenAccount>>,
    #[account(mut, associated_token::mint = mint_y, associated_token::authority = user)]
    pub user_y: Box<Account<'info, TokenAccount>>,

    #[account(mut, associated_token::mint = mint_lp, associated_token::authority = user)]
    pub user_lp: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> Withdraw<'info> {
    pub fn withdraw(&mut self, amount: u64, min_x: u64, min_y: u64) -> Result<()> {
        require!(!self.config.locked, AmmError::PoolLocked);
        require!(amount > 0, AmmError::ZeroAmount);

        let supply = self.mint_lp.supply;
        require!(supply > 0, AmmError::NoLiquidity);

        // round down so the pool keeps the remainder
        let x = floor_div(amount as u128, self.vault_x.amount as u128, supply as u128)?;
        let y = floor_div(amount as u128, self.vault_y.amount as u128, supply as u128)?;

        require!(x > 0 && y > 0, AmmError::InvalidOutput);
        require!(x >= min_x && y >= min_y, AmmError::SlippageExceeded);

        // burn before paying out
        self.burn_lp(amount)?;
        self.transfer_out(true, x)?;
        self.transfer_out(false, y)?;

        Ok(())
    }

    fn burn_lp(&self, amount: u64) -> Result<()> {
        let cpi = CpiContext::new(
            self.token_program.to_account_info(),
            Burn {
                mint: self.mint_lp.to_account_info(),
                from: self.user_lp.to_account_info(),
                authority: self.user.to_account_info(),
            },
        );
        token::burn(cpi, amount)
    }

    fn transfer_out(&self, is_x: bool, amount: u64) -> Result<()> {
        let (from, to) = if is_x {
            (&self.vault_x, &self.user_x)
        } else {
            (&self.vault_y, &self.user_y)
        };

        let seed = self.config.seed.to_le_bytes();
        let bump = [self.config.config_bump];
        let signer_seeds: &[&[&[u8]]] = &[&[CONFIG_SEED, seed.as_ref(), &bump]];

        let cpi = CpiContext::new_with_signer(
            self.token_program.to_account_info(),
            Transfer {
                from: from.to_account_info(),
                to: to.to_account_info(),
                authority: self.config.to_account_info(),
            },
            signer_seeds,
        );
        token::transfer(cpi, amount)
    }
}

// floor(amount * reserve / supply)
fn floor_div(amount: u128, reserve: u128, supply: u128) -> Result<u64> {
    let result = amount
        .checked_mul(reserve)
        .ok_or(AmmError::Overflow)?
        .checked_div(supply)
        .ok_or(AmmError::Overflow)?;
    u64::try_from(result).map_err(|_| AmmError::Overflow.into())
}
