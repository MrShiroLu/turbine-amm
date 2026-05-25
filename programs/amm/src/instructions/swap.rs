use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, Token, TokenAccount, Transfer},
};

use crate::{constants::*, error::AmmError, state::Config};

// is_x = true: pay x, get y. is_x = false: pay y, get x
#[derive(Accounts)]
pub struct Swap<'info> {
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

    #[account(mut, associated_token::mint = mint_x, associated_token::authority = config)]
    pub vault_x: Box<Account<'info, TokenAccount>>,
    #[account(mut, associated_token::mint = mint_y, associated_token::authority = config)]
    pub vault_y: Box<Account<'info, TokenAccount>>,

    #[account(mut, associated_token::mint = mint_x, associated_token::authority = user)]
    pub user_x: Box<Account<'info, TokenAccount>>,
    #[account(mut, associated_token::mint = mint_y, associated_token::authority = user)]
    pub user_y: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> Swap<'info> {
    pub fn swap(&mut self, is_x: bool, amount_in: u64, min_out: u64) -> Result<()> {
        require!(!self.config.locked, AmmError::PoolLocked);
        require!(amount_in > 0, AmmError::ZeroAmount);

        let (reserve_in, reserve_out) = if is_x {
            (self.vault_x.amount, self.vault_y.amount)
        } else {
            (self.vault_y.amount, self.vault_x.amount)
        };
        require!(reserve_in > 0 && reserve_out > 0, AmmError::NoLiquidity);

        let amount_out = constant_product_out(
            amount_in,
            reserve_in,
            reserve_out,
            self.config.fee,
        )?;

        require!(amount_out > 0 && amount_out < reserve_out, AmmError::InvalidOutput);
        require!(amount_out >= min_out, AmmError::SlippageExceeded);

        self.transfer_in(is_x, amount_in)?;
        self.transfer_out(is_x, amount_out)?; // pda-signed

        Ok(())
    }

    fn transfer_in(&self, is_x: bool, amount: u64) -> Result<()> {
        let (from, to) = if is_x {
            (&self.user_x, &self.vault_x)
        } else {
            (&self.user_y, &self.vault_y)
        };

        let cpi = CpiContext::new(
            self.token_program.to_account_info(),
            Transfer {
                from: from.to_account_info(),
                to: to.to_account_info(),
                authority: self.user.to_account_info(),
            },
        );
        token::transfer(cpi, amount)
    }

    fn transfer_out(&self, is_x: bool, amount: u64) -> Result<()> {
        // output is the opposite token
        let (from, to) = if is_x {
            (&self.vault_y, &self.user_y)
        } else {
            (&self.vault_x, &self.user_x)
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

// out = reserve_out * in_after_fee / (reserve_in + in_after_fee)
fn constant_product_out(
    amount_in: u64,
    reserve_in: u64,
    reserve_out: u64,
    fee_bps: u16,
) -> Result<u64> {
    let amount_in = amount_in as u128;
    let reserve_in = reserve_in as u128;
    let reserve_out = reserve_out as u128;

    let in_after_fee = amount_in
        .checked_mul(FEE_DENOMINATOR - fee_bps as u128)
        .ok_or(AmmError::Overflow)?
        .checked_div(FEE_DENOMINATOR)
        .ok_or(AmmError::Overflow)?;

    let numerator = reserve_out.checked_mul(in_after_fee).ok_or(AmmError::Overflow)?;
    let denominator = reserve_in.checked_add(in_after_fee).ok_or(AmmError::Overflow)?;
    let out = numerator.checked_div(denominator).ok_or(AmmError::Overflow)?;

    u64::try_from(out).map_err(|_| AmmError::Overflow.into())
}
