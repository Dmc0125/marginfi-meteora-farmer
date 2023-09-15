use std::sync::Arc;

use anchor_lang::{prelude::Pubkey, AccountDeserialize, Discriminator};
use base64::{engine::general_purpose, Engine};
use marginfi::state::marginfi_account::MarginfiAccount;
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_sdk::{account::Account, commitment_config::CommitmentConfig};
use tokio::task::JoinHandle;

use crate::{websocket::WebsocketClient, Error, Wallet};

pub mod marginfi_banks {
    pub mod bsol {
        use solana_sdk::declare_id;

        declare_id!("6hS9i46WyTq1KXcoa2Chas2Txh9TJAVr6n1t3tnrE23K");
    }

    pub mod uxd {
        use solana_sdk::declare_id;

        declare_id!("BeNBJrAh1tZg5sqgt8D6AWKJLD5KkBrfZvtcgd7EuiAR");
    }

    pub mod usdt {
        use solana_sdk::declare_id;

        declare_id!("HmpMfL8942u22htC4EMiWgLX931g3sacXFR6KjuLgKLV");
    }

    pub mod usdc {
        use solana_sdk::declare_id;

        declare_id!("4SryZ4bWGqEsNjbqNUKuxnoyagWgbxj6MavyUF2HRzhA");
    }
}

enum AccountData<'a> {
    Serialized(&'a Vec<u8>),
    Encoded(&'a UiAccountData),
}

impl<'a> From<&'a Account> for AccountData<'a> {
    fn from(value: &'a Account) -> Self {
        Self::Serialized(&value.data)
    }
}

impl<'a> From<&'a UiAccount> for AccountData<'a> {
    fn from(value: &'a UiAccount) -> Self {
        Self::Encoded(&value.data)
    }
}

impl<'a> AccountData<'a> {
    fn decode(encoded_data: &UiAccountData) -> Result<Vec<u8>, Error> {
        let res = match encoded_data {
            UiAccountData::Binary(encoded_data, encoding) => match encoding {
                UiAccountEncoding::Base64 => general_purpose::STANDARD.decode(encoded_data).ok(),
                _ => None,
            },
            _ => None,
        };
        res.ok_or(Error::UnableToDecode)
    }

    fn deserialize<T: AccountDeserialize + Discriminator>(data: &Vec<u8>) -> Result<T, Error> {
        T::try_deserialize(&mut &data[..]).map_err(|_| Error::UnableToDeserialize)
    }

    pub fn parse<T: AccountDeserialize + Discriminator>(&self) -> Result<T, Error> {
        match self {
            Self::Encoded(encoded) => {
                let bytes = Self::decode(encoded)?;
                Self::deserialize(&bytes)
            }
            Self::Serialized(bytes) => Self::deserialize(bytes),
        }
    }
}

pub enum Update {
    MarginfiUserAccount(MarginfiAccount),
    MarginfiBank(marginfi::state::marginfi_group::Bank),
}

pub type SubscriptionHandle = JoinHandle<Result<(), Error>>;

fn new_margin_fi_account_config(wallet: &Arc<Wallet>) -> RpcProgramAccountsConfig {
    RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
            40,
            wallet.pubkey.to_bytes().to_vec(),
        ))]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            data_slice: None,
            min_context_slot: None,
        },
        with_context: None,
    }
}

fn new_config_by_discriminator<T: Discriminator>() -> RpcProgramAccountsConfig {
    RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
            0,
            T::DISCRIMINATOR.to_vec(),
        ))]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            data_slice: None,
            min_context_slot: None,
        },
        with_context: None,
    }
}

pub async fn fetch_marginfi_account(
    rpc_client: &Arc<RpcClient>,
    wallet: &Arc<Wallet>,
) -> Result<MarginfiAccount, Error> {
    let config = new_margin_fi_account_config(wallet);

    let accounts = rpc_client
        .get_program_accounts_with_config(&marginfi::id(), config)
        .await?;

    if accounts.is_empty() {
        println!(
            "Marginfi account for {} does not exist",
            wallet.pubkey.to_string()
        );
        return Err(Error::UnableToFetchAccount);
    }

    AccountData::from(&accounts[0].1).parse()
}

pub async fn fetch_marginfi_banks(
    rpc_client: &Arc<RpcClient>,
) -> Result<Vec<(Pubkey, marginfi::state::marginfi_group::Bank)>, Error> {
    let config = new_config_by_discriminator::<marginfi::state::marginfi_group::Bank>();
    let accounts = rpc_client
        .get_program_accounts_with_config(&marginfi::id(), config)
        .await?;

    accounts
        .iter()
        .map(|(address, account)| {
            let bank = AccountData::from(account).parse();
            bank.map(|bank| (*address, bank))
        })
        .collect()
}
