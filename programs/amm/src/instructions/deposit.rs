use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::AssociatedToken,
    token::{self, Mint, MintTo, Token, TokenAccount, Transfer},
};

use crate::{constants::*, error::AmmError, state::Config};

// add liquidity, mint LP. max_x/max_y are slippage caps
#[derive(Accounts)]
pub struct Deposit<'info> {
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

    #[account(
        init_if_needed,
        payer = user,
        associated_token::mint = mint_lp,
        associated_token::authority = user,
    )]
    pub user_lp: Box<Account<'info, TokenAccount>>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> Deposit<'info> {
    pub fn deposit(&mut self, amount: u64, max_x: u64, max_y: u64) -> Result<()> {
        require!(!self.config.locked, AmmError::PoolLocked);
        require!(amount > 0, AmmError::ZeroAmount);

        let supply = self.mint_lp.supply;

        let (x, y) = if supply == 0 {
            // first deposit sets the price
            require!(max_x > 0 && max_y > 0, AmmError::ZeroAmount);
            (max_x, max_y)
        } else {
            // round up so rounding favours the pool, not the depositor
            let x = ceil_div(amount as u128, self.vault_x.amount as u128, supply as u128)?;
            let y = ceil_div(amount as u128, self.vault_y.amount as u128, supply as u128)?;
            require!(x <= max_x && y <= max_y, AmmError::SlippageExceeded);
            (x, y)
        };

        require!(x > 0 && y > 0, AmmError::ZeroAmount);

        self.transfer_to_vault(true, x)?;
        self.transfer_to_vault(false, y)?;
        self.mint_lp_tokens(amount)?;

        Ok(())
    }

    fn transfer_to_vault(&self, is_x: bool, amount: u64) -> Result<()> {
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

    fn mint_lp_tokens(&self, amount: u64) -> Result<()> {
        let seed = self.config.seed.to_le_bytes();
        let bump = [self.config.config_bump];
        let signer_seeds: &[&[&[u8]]] = &[&[CONFIG_SEED, seed.as_ref(), &bump]];

        let cpi = CpiContext::new_with_signer(
            self.token_program.to_account_info(),
            MintTo {
                mint: self.mint_lp.to_account_info(),
                to: self.user_lp.to_account_info(),
                authority: self.config.to_account_info(),
            },
            signer_seeds,
        );
        token::mint_to(cpi, amount)
    }
}

// ceil(amount * reserve / supply)
fn ceil_div(amount: u128, reserve: u128, supply: u128) -> Result<u64> {
    let numerator = amount.checked_mul(reserve).ok_or(AmmError::Overflow)?;
    let result = numerator
        .checked_add(supply - 1)
        .ok_or(AmmError::Overflow)?
        .checked_div(supply)
        .ok_or(AmmError::Overflow)?;
    u64::try_from(result).map_err(|_| AmmError::Overflow.into())
}
