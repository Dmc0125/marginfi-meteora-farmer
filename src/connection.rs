use std::{str::FromStr, sync::Arc, time::SystemTime};

use anchor_lang::{
    prelude::{AccountMeta, Pubkey},
    AccountDeserialize, Discriminator,
};
use base64::{engine::general_purpose, Engine};
use futures_util::StreamExt;
use marginfi::{constants::PYTH_ID, state::marginfi_account::MarginfiAccount};
use serde::{de::Visitor, Deserialize};
use solana_account_decoder::{UiAccount, UiAccountData, UiAccountEncoding};
use solana_client::{
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_sdk::{
    account::Account, address_lookup_table_account::AddressLookupTableAccount,
    commitment_config::CommitmentConfig, instruction::Instruction,
};
use switchboard_v2::AggregatorAccountData;
use tokio::{sync::mpsc, task::JoinHandle};

use crate::{
    addresses::{MarginfiBank, MarginfiBankOracle},
    constants,
    state::{PythPriceFeed, StateUpdate, SwitchboardPriceFeed},
    utils::websocket_client::WebsocketClient,
    Error, Wallet,
};

pub fn decode_base64_data(encoded: &String) -> Option<Vec<u8>> {
    general_purpose::STANDARD.decode(encoded).ok()
}

pub enum AccountData<'a> {
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
    pub fn decode(encoded_data: &UiAccountData) -> Result<Vec<u8>, Error> {
        let res = match encoded_data {
            UiAccountData::Binary(encoded_data, encoding) => match encoding {
                UiAccountEncoding::Base64 => decode_base64_data(encoded_data),
                _ => None,
            },
            _ => None,
        };
        res.ok_or(Error::UnableToDecode)
    }

    pub fn deserialize<T: AccountDeserialize + Discriminator>(data: &Vec<u8>) -> Result<T, Error> {
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

fn new_config_by_discriminator(
    discriminator: Vec<u8>,
    filters: Option<Vec<RpcFilterType>>,
) -> RpcProgramAccountsConfig {
    let mut config = RpcProgramAccountsConfig {
        filters: Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
            0,
            discriminator,
        ))]),
        account_config: RpcAccountInfoConfig {
            encoding: Some(UiAccountEncoding::Base64),
            commitment: Some(CommitmentConfig::confirmed()),
            data_slice: None,
            min_context_slot: None,
        },
        with_context: None,
    };
    if let Some(filters) = filters {
        let cf = &mut config.filters;
        if let Some(cf) = cf {
            filters.iter().for_each(|f| {
                cf.push(f.clone());
            });
        }
    }
    config
}

pub struct MeteoraPoolsAndVaults {
    pub pools: Vec<(Pubkey, meteora::state::Pool)>,
    pub vaults: Vec<(Pubkey, meteora_vault::state::Vault)>,
}

pub async fn fetch_meteora_pools_and_vaults(
    rpc_client: &Arc<RpcClient>,
) -> Result<MeteoraPoolsAndVaults, Error> {
    let pools_addresses = vec![constants::meteora::acusd_usdc_pool::id()];
    let mut vaults_addresses = vec![];

    let mut pools_and_vaults = MeteoraPoolsAndVaults {
        pools: vec![],
        vaults: vec![],
    };

    let pools_ais = rpc_client.get_multiple_accounts(&pools_addresses).await?;

    for (i, ai) in pools_ais.iter().enumerate() {
        let address = pools_addresses[i];

        if let Some(ai) = ai {
            let pool: meteora::state::Pool = AccountData::from(ai).parse()?;

            if !vaults_addresses.contains(&pool.a_vault) {
                vaults_addresses.push(pool.a_vault);
            }
            if !vaults_addresses.contains(&pool.b_vault) {
                vaults_addresses.push(pool.b_vault);
            }

            pools_and_vaults.pools.push((address, pool));
        } else {
            println!("Meteora pool does not exist: {}", address);
            return Err(Error::UnableToFetchAccount);
        }
    }

    let vaults_ais = rpc_client.get_multiple_accounts(&vaults_addresses).await?;

    for (i, ai) in vaults_ais.iter().enumerate() {
        let address = vaults_addresses[i];

        if let Some(ai) = ai {
            pools_and_vaults
                .vaults
                .push((address, AccountData::from(ai).parse()?))
        } else {
            println!("Meteora vault does not exist: {}", address);
            return Err(Error::UnableToFetchAccount);
        }
    }

    Ok(pools_and_vaults)
}

pub async fn fetch_marginfi_account(
    rpc_client: &Arc<RpcClient>,
    wallet: &Arc<Wallet>,
) -> Result<(Pubkey, MarginfiAccount), Error> {
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

    Ok((accounts[0].0, AccountData::from(&accounts[0].1).parse()?))
}

pub async fn fetch_marginfi_banks(
    rpc_client: &Arc<RpcClient>,
) -> Result<Vec<(Pubkey, marginfi::state::marginfi_group::Bank)>, Error> {
    let config = new_config_by_discriminator(
        marginfi::state::marginfi_group::Bank::DISCRIMINATOR.to_vec(),
        Some(vec![RpcFilterType::Memcmp(Memcmp::new_raw_bytes(
            41,
            constants::marginfi::group::id().to_bytes().to_vec(),
        ))]),
    );
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

pub fn subscribe_to_pyth_oracles(
    ws_client: Arc<WebsocketClient>,
    banks: &Vec<(Pubkey, MarginfiBank)>,
    state_update_sender: mpsc::UnboundedSender<StateUpdate>,
) -> SubscriptionHandle {
    let magic = pyth_sdk_solana::state::MAGIC.to_le_bytes();
    let config = new_config_by_discriminator(magic.to_vec(), None);
    let watched_oracles = banks
        .iter()
        .filter_map(|(_, bank)| match bank.oracle {
            MarginfiBankOracle::Pyth(addr) => Some(addr),
            _ => None,
        })
        .collect::<Vec<Pubkey>>();

    tokio::spawn(async move {
        loop {
            let (_, mut stream) = ws_client.program_subscribe(PYTH_ID, config.clone()).await?;

            while let Some(payload) = stream.next().await {
                let pubkey = Pubkey::from_str(&payload.value.pubkey).unwrap();

                if !watched_oracles.contains(&pubkey) {
                    continue;
                }

                let bytes = AccountData::decode(&payload.value.account.data).unwrap();
                let price_feed = pyth_sdk_solana::state::load_price_account(&bytes[..])
                    .unwrap()
                    .to_price_feed(&pubkey);
                let now_ts = SystemTime::now()
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .unwrap()
                    .as_secs();

                if let Some(price) = price_feed.get_ema_price_no_older_than(now_ts as i64, 60) {
                    let price_feed = PythPriceFeed {
                        price,
                        last_update_slot: payload.context.slot,
                    };
                    state_update_sender
                        .send(StateUpdate::PythOracle((pubkey, price_feed)))
                        .ok();
                }
            }
        }
    })
}

pub async fn init_and_subscribe_to_switchboard_oracles(
    rpc_client: Arc<RpcClient>,
    ws_client: Arc<WebsocketClient>,
    banks: &Vec<(Pubkey, MarginfiBank)>,
    state_update_sender: mpsc::UnboundedSender<StateUpdate>,
) -> Result<SubscriptionHandle, Error> {
    let config = new_config_by_discriminator(AggregatorAccountData::DISCRIMINATOR.to_vec(), None);
    let watched_oracles = banks
        .iter()
        .filter_map(|(_, bank)| match bank.oracle {
            MarginfiBankOracle::Switchboard(addr) => Some(addr),
            _ => None,
        })
        .collect::<Vec<Pubkey>>();

    let accounts = rpc_client.get_multiple_accounts(&watched_oracles).await?;
    for (i, ai) in accounts.iter().enumerate() {
        if let Some(ai) = ai {
            let pubkey = &watched_oracles[i];
            let aggregator_account = AccountData::from(ai)
                .parse::<AggregatorAccountData>()
                .unwrap();
            let price_feed = SwitchboardPriceFeed::from(&aggregator_account);

            state_update_sender
                .send(StateUpdate::SwitchboardOracle((*pubkey, price_feed)))
                .ok();
        } else {
            return Err(Error::UnableToFetchAccount);
        }
    }

    let handle = tokio::spawn(async move {
        loop {
            let (_, mut stream) = ws_client
                .program_subscribe(switchboard_v2::SWITCHBOARD_V2_MAINNET, config.clone())
                .await?;

            while let Some(payload) = stream.next().await {
                let pubkey = Pubkey::from_str(&payload.value.pubkey).unwrap();

                if !watched_oracles.contains(&pubkey) {
                    continue;
                }

                let aggregator_account = AccountData::from(&payload.value.account)
                    .parse::<AggregatorAccountData>()
                    .unwrap();
                let price_feed = SwitchboardPriceFeed::from(&aggregator_account);

                state_update_sender
                    .send(StateUpdate::SwitchboardOracle((pubkey, price_feed)))
                    .ok();
            }
        }
    });
    Ok(handle)
}

struct PubkeyVisitor;

impl<'de> Visitor<'de> for PubkeyVisitor {
    type Value = PubkeyDe;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("Invalid pubkey")
    }

    fn visit_string<E>(self, v: String) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(PubkeyDe(
            Pubkey::from_str(&v).map_err(|e| E::custom(e.to_string()))?,
        ))
    }

    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(PubkeyDe(
            Pubkey::from_str(v).map_err(|e| E::custom(e.to_string()))?,
        ))
    }
}

#[derive(Debug, Copy, Clone)]
struct PubkeyDe(pub Pubkey);

impl<'de> Deserialize<'de> for PubkeyDe {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_string(PubkeyVisitor)
    }
}

#[derive(Debug, Deserialize, Copy, Clone)]
#[serde(rename_all = "camelCase")]
struct JupiterAccount {
    pubkey: PubkeyDe,
    is_signer: bool,
    is_writable: bool,
}

impl Into<AccountMeta> for JupiterAccount {
    fn into(self) -> AccountMeta {
        AccountMeta {
            pubkey: self.pubkey.0,
            is_signer: self.is_signer,
            is_writable: self.is_writable,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterInstruction {
    program_id: PubkeyDe,
    accounts: Vec<JupiterAccount>,
    data: String,
}

impl Into<Instruction> for JupiterInstruction {
    fn into(self) -> Instruction {
        let bytes = decode_base64_data(&self.data).unwrap();
        Instruction {
            program_id: self.program_id.0,
            accounts: self.accounts.iter().map(|a| (*a).into()).collect(),
            data: bytes,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JupiterIxsResponse {
    pub compute_budget_instructions: Option<Vec<JupiterInstruction>>,
    pub setup_instructions: Option<Vec<JupiterInstruction>>,
    pub swap_instruction: JupiterInstruction,
    pub cleanup_instruction: Option<Vec<JupiterInstruction>>,
    pub address_lookup_table_addresses: Vec<String>,
}

impl Into<Vec<Instruction>> for JupiterIxsResponse {
    fn into(self) -> Vec<Instruction> {
        let mut ixs = vec![];

        if let Some(cbi) = self.compute_budget_instructions {
            for ix in cbi {
                ixs.push(ix.into());
            }
        }
        if let Some(si) = self.setup_instructions {
            for ix in si {
                ixs.push(ix.into())
            }
        }
        ixs.push(self.swap_instruction.into());
        if let Some(ci) = self.cleanup_instruction {
            for ix in ci {
                ixs.push(ix.into())
            }
        }

        ixs
    }
}

pub async fn fetch_swap_instructions(
    rpc_client: &Arc<RpcClient>,
    client: &reqwest::Client,
    wallet: &Arc<Wallet>,
    input_mint: &Pubkey,
    input_amount: u64,
) -> Result<(Vec<Instruction>, Vec<AddressLookupTableAccount>), Error> {
    const API_URL: &'static str = "https://quote-api.jup.ag/v6";

    let get_url_params = format!(
        "?inputMint={}&outputMint={}&amount={}&slippageBps=10&onlyDirectRoutes=false&asLegacyTransaction=false",
        input_mint.to_string(),
        constants::mints::usdc::id().to_string(),
        input_amount,
    );
    let quote_res = client
        .get(format!("{API_URL}/quote{get_url_params}"))
        .send()
        .await?
        .text()
        .await?;

    let body = format!(
        "{{\"userPublicKey\":\"{}\",\"quoteResponse\":{quote_res}}}",
        wallet.pubkey.to_string()
    );
    let res = client
        .post(format!("{API_URL}/swap-instructions"))
        .body(body)
        .send()
        .await?
        .json::<JupiterIxsResponse>()
        .await?;

    let alt_addresses = res
        .address_lookup_table_addresses
        .iter()
        .map(|str| Pubkey::from_str(str).unwrap())
        .collect::<Vec<Pubkey>>();
    let alt_ais = rpc_client.get_multiple_accounts(&alt_addresses).await?;
    let mut alt_accounts: Vec<AddressLookupTableAccount> = vec![];
    for (i, ai) in alt_ais.iter().enumerate() {
        if let Some(ai) = ai {
            let alt = solana_address_lookup_table_program::state::AddressLookupTable::deserialize(
                &ai.data,
            );
            if let Ok(alt) = alt {
                alt_accounts.push(AddressLookupTableAccount {
                    key: alt_addresses[i],
                    addresses: alt.addresses.to_vec(),
                });
            }
        }
    }

    let instructions: Vec<Instruction> = res.into();

    Ok((instructions, alt_accounts))
}
