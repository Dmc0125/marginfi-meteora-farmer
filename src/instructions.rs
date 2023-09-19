use std::sync::Arc;

use anchor_lang::{
    prelude::{borsh, AccountMeta, Pubkey},
    AnchorSerialize, Discriminator,
};
use solana_sdk::instruction::Instruction;

use crate::{
    addresses::{MeteoraDynamicPool, StaticAddresses},
    constants,
    state::MarginfiAccountWithBanks,
    Error, Wallet,
};

#[derive(AnchorSerialize)]
struct AnchorIxData<T: AnchorSerialize> {
    discriminator: [u8; 8],
    data: T,
}

#[derive(AnchorSerialize)]
struct MeteoraDeposit {
    minimum_pool_token_amount: u64,
    token_a_amount: u64,
    token_b_amount: u64,
}

pub struct InstructionBuilder {
    wallet: Arc<Wallet>,
}

impl InstructionBuilder {
    pub fn new(wallet: Arc<Wallet>) -> Self {
        Self { wallet }
    }

    pub fn marginfi_deposit(
        &self,
        static_addresses: &StaticAddresses,
        mint: &Pubkey,
        amount: u64,
        marginfi_account: &MarginfiAccountWithBanks,
    ) -> Result<Instruction, Error> {
        let data = AnchorIxData {
            discriminator: marginfi::instruction::LendingAccountDeposit::DISCRIMINATOR,
            data: amount,
        };

        let bank_accounts = static_addresses.get_marginfi_bank(mint)?;
        let token_account = static_addresses.get_token_account(mint)?;

        let mut accounts: Vec<AccountMeta> = vec![
            AccountMeta::new_readonly(constants::marginfi::group::id(), false),
            AccountMeta::new(static_addresses.marginfi_account, false),
            AccountMeta::new(self.wallet.pubkey, true),
            AccountMeta::new(bank_accounts.address, false),
            AccountMeta::new(token_account, false),
            AccountMeta::new(bank_accounts.liquidity_vault, false),
            AccountMeta::new_readonly(constants::spl_token::id(), false),
        ];

        marginfi_account.balances.iter().for_each(|(_, balance)| {
            if balance.is_active {
                if let Ok(bank) =
                    static_addresses.get_marginfi_bank_by_bank_address(&balance.bank_address)
                {
                    accounts.push(AccountMeta::new_readonly(bank.address, false));
                    accounts.push(AccountMeta::new_readonly(bank.oracle.address(), false));
                }
            }
        });

        Ok(Instruction::new_with_borsh(marginfi::id(), &data, accounts))
    }

    pub fn marginfi_borrow(
        &self,
        static_addresses: &StaticAddresses,
        mint: &Pubkey,
        amount: u64,
        marginfi_account: &MarginfiAccountWithBanks,
    ) -> Result<Instruction, Error> {
        let data = AnchorIxData {
            discriminator: marginfi::instruction::LendingAccountBorrow::DISCRIMINATOR,
            data: amount,
        };

        let bank_accounts = static_addresses.get_marginfi_bank(mint)?;
        let token_account = static_addresses.get_token_account(mint)?;

        let mut accounts: Vec<AccountMeta> = vec![
            AccountMeta::new_readonly(constants::marginfi::group::id(), false),
            AccountMeta::new(static_addresses.marginfi_account, false),
            AccountMeta::new(self.wallet.pubkey, true),
            AccountMeta::new(bank_accounts.address, false),
            AccountMeta::new(token_account, false),
            AccountMeta::new(bank_accounts.liquidity_vault_authority, false),
            AccountMeta::new(bank_accounts.liquidity_vault, false),
            AccountMeta::new_readonly(constants::spl_token::id(), false),
        ];

        marginfi_account.balances.iter().for_each(|(_, balance)| {
            if balance.is_active {
                if let Ok(bank) =
                    static_addresses.get_marginfi_bank_by_bank_address(&balance.bank_address)
                {
                    accounts.push(AccountMeta::new_readonly(bank.address, false));
                    accounts.push(AccountMeta::new_readonly(bank.oracle.address(), false));
                }
            }
        });

        Ok(Instruction::new_with_borsh(marginfi::id(), &data, accounts))
    }

    pub fn meteora_pool_deposit(
        &self,
        static_addresses: &StaticAddresses,
        pool: &MeteoraDynamicPool,
        minimum_pool_token_amount: u64,
        token_a_amount: u64,
        token_b_amount: u64,
    ) -> Result<Instruction, Error> {
        let data = AnchorIxData {
            discriminator: meteora::instruction::AddBalanceLiquidity::DISCRIMINATOR,
            data: MeteoraDeposit {
                minimum_pool_token_amount,
                token_a_amount,
                token_b_amount,
            },
        };

        let lp_token_account = static_addresses.get_token_account(&pool.lp_mint)?;
        let a_token_account = static_addresses.get_token_account(&pool.a_token_mint)?;
        let b_token_account = static_addresses.get_token_account(&pool.b_token_mint)?;

        let accounts = vec![
            AccountMeta::new(pool.address, false),
            AccountMeta::new(pool.lp_mint, false),
            AccountMeta::new(lp_token_account, false),
            AccountMeta::new(pool.a_vault_lp, false),
            AccountMeta::new(pool.b_vault_lp, false),
            AccountMeta::new(pool.a_vault, false),
            AccountMeta::new(pool.b_vault, false),
            AccountMeta::new(pool.vault_a_lp_mint, false),
            AccountMeta::new(pool.vault_b_lp_mint, false),
            AccountMeta::new(pool.vault_a_vault, false),
            AccountMeta::new(pool.vault_b_vault, false),
            AccountMeta::new(a_token_account, false),
            AccountMeta::new(b_token_account, false),
            AccountMeta::new(self.wallet.pubkey, true),
            AccountMeta::new_readonly(meteora_vault::id(), false),
            AccountMeta::new_readonly(constants::spl_token::id(), false),
        ];

        Ok(Instruction::new_with_borsh(meteora::id(), &data, accounts))
    }

    fn generate_discriminator(preimage: &'static str) -> [u8; 8] {
        let mut discriminator = [0u8; 8];

        let bytes = solana_sdk::hash::hash(&preimage.as_bytes()).to_bytes();
        discriminator.copy_from_slice(&bytes[..8]);

        discriminator
    }

    pub fn meteora_farm_deposit(
        &self,
        static_addresses: &StaticAddresses,
        mint: &Pubkey,
        amount: u64,
    ) -> Result<Instruction, Error> {
        let data = AnchorIxData {
            discriminator: Self::generate_discriminator("global:deposit"),
            data: amount,
        };

        let farm = static_addresses.get_meteora_farm(mint)?;
        let pool = static_addresses.get_meteora_pool(mint)?;
        let lp_token_account = static_addresses.get_token_account(&pool.lp_mint)?;

        let accounts = vec![
            AccountMeta::new(farm.address, false),
            AccountMeta::new(farm.staking_vault, false),
            AccountMeta::new(farm.user_account, false),
            AccountMeta::new(self.wallet.pubkey, true),
            AccountMeta::new(lp_token_account, false),
            AccountMeta::new_readonly(constants::spl_token::id(), false),
        ];

        Ok(Instruction::new_with_borsh(
            constants::meteora::farm::id(),
            &data,
            accounts,
        ))
    }
}
