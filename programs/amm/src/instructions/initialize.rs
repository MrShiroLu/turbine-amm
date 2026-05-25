use anchor_lang::prelude::*;
use anchor_spl::{
    associated_token::{self, AssociatedToken, Create},
    token::{Mint, Token},
};

use crate::{constants::*, error::AmmError, state::Config};

// vaults are created in the handler (not via init) to keep try_accounts
// under the SBF stack limit
#[derive(Accounts)]
#[instruction(seed: u64)]
pub struct Initialize<'info> {
    #[account(mut)]
    pub initializer: Signer<'info>,

    pub mint_x: Box<Account<'info, Mint>>,
    pub mint_y: Box<Account<'info, Mint>>,

    #[account(
        init,
        payer = initializer,
        seeds = [CONFIG_SEED, seed.to_le_bytes().as_ref()],
        bump,
        space = 8 + Config::INIT_SPACE,
    )]
    pub config: Box<Account<'info, Config>>,

    #[account(
        init,
        payer = initializer,
        seeds = [LP_SEED, config.key().as_ref()],
        bump,
        mint::decimals = LP_DECIMALS,
        mint::authority = config,
    )]
    pub mint_lp: Box<Account<'info, Mint>>,

    #[account(mut)]
    pub vault_x: UncheckedAccount<'info>,
    #[account(mut)]
    pub vault_y: UncheckedAccount<'info>,

    pub token_program: Program<'info, Token>,
    pub associated_token_program: Program<'info, AssociatedToken>,
    pub system_program: Program<'info, System>,
}

impl<'info> Initialize<'info> {
    pub fn initialize(
        &mut self,
        seed: u64,
        fee: u16,
        authority: Option<Pubkey>,
        bumps: &InitializeBumps,
    ) -> Result<()> {
        require!(fee <= 10_000, AmmError::InvalidFee);
        require_keys_neq!(
            self.mint_x.key(),
            self.mint_y.key(),
            AmmError::IdenticalMints
        );

        self.create_vault(self.vault_x.to_account_info(), self.mint_x.to_account_info())?;
        self.create_vault(self.vault_y.to_account_info(), self.mint_y.to_account_info())?;

        self.config.set_inner(Config {
            seed,
            authority,
            mint_x: self.mint_x.key(),
            mint_y: self.mint_y.key(),
            fee,
            locked: false,
            config_bump: bumps.config,
            lp_bump: bumps.mint_lp,
        });

        Ok(())
    }

    fn create_vault(&self, vault: AccountInfo<'info>, mint: AccountInfo<'info>) -> Result<()> {
        let cpi = CpiContext::new(
            self.associated_token_program.to_account_info(),
            Create {
                payer: self.initializer.to_account_info(),
                associated_token: vault,
                authority: self.config.to_account_info(),
                mint,
                system_program: self.system_program.to_account_info(),
                token_program: self.token_program.to_account_info(),
            },
        );
        associated_token::create(cpi)
    }
}
