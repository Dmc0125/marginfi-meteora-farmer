use std::sync::Arc;

use anchor_lang::prelude::Pubkey;
use marginfi::state::price::OracleSetup;

use crate::{connection::MeteoraPoolsAndVaults, constants, Error, Wallet};

pub enum MarginfiBankOracle {
    Pyth(Pubkey),
    Switchboard(Pubkey),
}

impl MarginfiBankOracle {
    pub fn address(&self) -> Pubkey {
        match self {
            Self::Pyth(addres) => *addres,
            Self::Switchboard(address) => *address,
        }
    }
}

pub struct MarginfiBank {
    pub address: Pubkey,
    pub liquidity_vault: Pubkey,
    pub liquidity_vault_authority: Pubkey,
    pub oracle: MarginfiBankOracle,
}

pub struct MeteoraDynamicPool {
    pub address: Pubkey,

    // Pool
    pub lp_mint: Pubkey,
    pub a_vault: Pubkey,
    pub b_vault: Pubkey,
    pub a_vault_lp: Pubkey,
    pub b_vault_lp: Pubkey,

    // Vault
    pub vault_a_vault: Pubkey,
    pub vault_b_vault: Pubkey,
    pub vault_a_lp_mint: Pubkey,
    pub vault_b_lp_mint: Pubkey,

    pub a_token_mint: Pubkey,
    pub b_token_mint: Pubkey,
}

impl MeteoraDynamicPool {
    pub fn get_token_for_deposit(&self, amount: u64, mint: &Pubkey) -> (u64, u64) {
        if mint == &self.a_token_mint {
            (amount, 0)
        } else {
            (0, amount)
        }
    }
}

pub struct MeteoraFarmMeta {
    pub address: Pubkey,
    pub staking_vault: Pubkey,
    pub user_account: Pubkey,
}

pub struct StaticAddresses {
    pub wallet_token_accounts: Vec<(Pubkey, Pubkey)>,
    pub marginfi_account: Pubkey,
    pub marginfi_banks: Vec<(Pubkey, MarginfiBank)>,
    // key: input mint
    pub meteora_dynamic_pools: Vec<(Pubkey, MeteoraDynamicPool)>,
    // key: pool input mint
    pub meteora_farms: Vec<(Pubkey, MeteoraFarmMeta)>,
}

impl StaticAddresses {
    pub fn new(wallet: &Arc<Wallet>) -> Self {
        let mut token_accounts = vec![];
        for mint in [
            constants::mints::bsol::id(),
            constants::mints::usdc::id(),
            constants::mints::uxd::id(),
            constants::mints::usdt::id(),
        ] {
            let token_account_address = Pubkey::find_program_address(
                &[
                    wallet.pubkey.as_ref(),
                    constants::spl_token::id().as_ref(),
                    mint.as_ref(),
                ],
                &constants::associated_token::id(),
            )
            .0;
            token_accounts.push((mint, token_account_address));
        }

        Self {
            wallet_token_accounts: token_accounts,
            marginfi_account: Pubkey::default(),
            marginfi_banks: vec![],
            meteora_dynamic_pools: vec![],
            meteora_farms: vec![],
        }
    }

    pub fn set_marginfi_account(mut self, marginfi_account: Pubkey) -> Self {
        self.marginfi_account = marginfi_account;
        self
    }

    pub fn set_marginfi_banks(
        mut self,
        banks: &Vec<(Pubkey, marginfi::state::marginfi_group::Bank)>,
    ) -> Self {
        banks.iter().for_each(|(bank_address, bank)| {
            let mint = bank.mint;
            let oracle_address = bank.config.oracle_keys[0];
            let oracle = match bank.config.oracle_setup {
                OracleSetup::PythEma => MarginfiBankOracle::Pyth(oracle_address),
                OracleSetup::SwitchboardV2 => MarginfiBankOracle::Switchboard(oracle_address),
                OracleSetup::None => unreachable!(),
            };
            let liquidity_vault_authority = Pubkey::find_program_address(
                &[
                    marginfi::constants::LIQUIDITY_VAULT_AUTHORITY_SEED.as_bytes(),
                    bank_address.as_ref(),
                ],
                &marginfi::id(),
            )
            .0;
            self.marginfi_banks.push((
                mint,
                MarginfiBank {
                    address: *bank_address,
                    liquidity_vault: bank.liquidity_vault,
                    liquidity_vault_authority,
                    oracle,
                },
            ));
        });
        self
    }

    fn add_unique_wallet_token_account(&mut self, mint: &Pubkey, wallet: &Arc<Wallet>) {
        let token_account = Pubkey::find_program_address(
            &[
                wallet.pubkey.as_ref(),
                constants::spl_token::id().as_ref(),
                mint.as_ref(),
            ],
            &constants::associated_token::id(),
        )
        .0;

        if !self.wallet_token_accounts.contains(&(*mint, token_account)) {
            self.wallet_token_accounts.push((*mint, token_account));
        }
    }

    fn get_meteora_pool_input_mint(pool: &Pubkey) -> Result<Pubkey, Error> {
        if pool == &constants::meteora::acusd_usdc_pool::id() {
            Ok(constants::mints::usdc::id())
        } else {
            Err(Error::InvalidMeteoraPool)
        }
    }

    pub fn set_meteora_pools_and_vaults(
        mut self,
        wallet: &Arc<Wallet>,
        pools_and_vaults: &MeteoraPoolsAndVaults,
    ) -> Result<Self, Error> {
        for (pool_address, pool) in pools_and_vaults.pools.iter() {
            let input_mint = Self::get_meteora_pool_input_mint(&pool_address)?;

            let (_, a_vault) = pools_and_vaults
                .vaults
                .iter()
                .find(|(addr, _)| addr == &pool.a_vault)
                .unwrap();
            let (_, b_vault) = pools_and_vaults
                .vaults
                .iter()
                .find(|(addr, _)| addr == &pool.b_vault)
                .unwrap();

            self.add_unique_wallet_token_account(&pool.token_a_mint, wallet);
            self.add_unique_wallet_token_account(&pool.token_b_mint, wallet);
            self.add_unique_wallet_token_account(&pool.lp_mint, wallet);

            self.meteora_dynamic_pools.push((
                input_mint,
                MeteoraDynamicPool {
                    address: *pool_address,
                    lp_mint: pool.lp_mint,
                    a_vault: pool.a_vault,
                    b_vault: pool.b_vault,
                    a_vault_lp: pool.a_vault_lp,
                    b_vault_lp: pool.b_vault_lp,
                    a_token_mint: pool.token_a_mint,
                    b_token_mint: pool.token_b_mint,
                    vault_a_vault: a_vault.token_vault,
                    vault_b_vault: b_vault.token_vault,
                    vault_a_lp_mint: a_vault.lp_mint,
                    vault_b_lp_mint: b_vault.lp_mint,
                },
            ));
        }

        Ok(self)
    }

    pub fn set_meteora_farms(mut self, wallet: &Arc<Wallet>) -> Self {
        let farm_address = constants::meteora::acusd_usdc_farm::id();
        let user_account = Pubkey::find_program_address(
            &[wallet.pubkey.as_ref(), farm_address.as_ref()],
            &constants::meteora::farm::id(),
        )
        .0;
        let staking_vault = Pubkey::find_program_address(
            &[b"staking", farm_address.as_ref()],
            &constants::meteora::farm::id(),
        )
        .0;

        self.meteora_farms.push((
            constants::mints::usdc::id(),
            MeteoraFarmMeta {
                address: farm_address,
                user_account,
                staking_vault,
            },
        ));

        self
    }

    pub fn get_marginfi_bank(&self, mint: &Pubkey) -> Result<&MarginfiBank, Error> {
        self.marginfi_banks
            .iter()
            .find(|(bank_mint, _)| bank_mint == mint)
            .map(|(_, bank)| bank)
            .ok_or(Error::InvalidMarginfiBank)
    }

    pub fn get_marginfi_bank_by_bank_address(
        &self,
        address: &Pubkey,
    ) -> Result<&MarginfiBank, Error> {
        self.marginfi_banks
            .iter()
            .find(|(_, bank)| &bank.address == address)
            .map(|(_, bank)| bank)
            .ok_or(Error::InvalidMarginfiBank)
    }

    pub fn get_token_account(&self, mint: &Pubkey) -> Result<Pubkey, Error> {
        self.wallet_token_accounts
            .iter()
            .find(|(token_mint, _)| token_mint == mint)
            .map(|(_, token_account)| *token_account)
            .ok_or(Error::InvalidTokenAccount)
    }

    pub fn get_meteora_pool(&self, mint: &Pubkey) -> Result<&MeteoraDynamicPool, Error> {
        self.meteora_dynamic_pools
            .iter()
            .find(|(inpt_mint, _)| inpt_mint == mint)
            .map(|(_, p)| p)
            .ok_or(Error::InvalidMeteoraPool)
    }

    pub fn get_meteora_farm(&self, mint: &Pubkey) -> Result<&MeteoraFarmMeta, Error> {
        self.meteora_farms
            .iter()
            .find(|(inpt_mint, _)| inpt_mint == mint)
            .map(|(_, p)| p)
            .ok_or(Error::InvalidMeteoraFarm)
    }
}
