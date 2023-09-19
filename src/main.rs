use std::{sync::Arc, time::Duration};

use anchor_lang::prelude::Pubkey;
use args::Args;
use connection::{fetch_marginfi_account, fetch_marginfi_banks};
use solana_client::client_error::ClientError;
use solana_sdk::signature::Keypair;
use state::OraclesState;
use tokio::{sync::mpsc, time::sleep};
use utils::transaction::ClientTransactionError;

use crate::{
    addresses::StaticAddresses,
    connection::fetch_meteora_pools_and_vaults,
    instructions::InstructionBuilder,
    utils::websocket_client::{create_persisted_websocket_connection, WebsocketError},
};

pub mod addresses;
pub mod args;
pub mod bot;
pub mod connection;
pub mod constants;
pub mod instructions;
pub mod state;
pub mod utils;

#[derive(Debug)]
pub struct Wallet {
    pub keypair: Keypair,
    pub pubkey: Pubkey,
}

#[derive(Debug)]
pub enum Error {
    UnableToDecode,
    UnableToDeserialize,
    UnableToFetchAccount,
    UnableToParsePythOracle,
    UnableToParseSwitchboardOracle,

    InvalidMarginfiBank,
    InvalidTokenAccount,
    InvalidMeteoraPool,
    InvalidMeteoraFarm,

    TransactionError,

    MathOverflow,
    ClientTransactionError(ClientTransactionError),

    JupiterApiError(reqwest::Error),
    RpcError,
    WebsocketError(WebsocketError),
}

impl From<ClientError> for Error {
    fn from(_: ClientError) -> Self {
        Self::RpcError
    }
}

impl From<WebsocketError> for Error {
    fn from(value: WebsocketError) -> Self {
        Self::WebsocketError(value)
    }
}

impl From<ClientTransactionError> for Error {
    fn from(value: ClientTransactionError) -> Self {
        Self::ClientTransactionError(value)
    }
}

impl From<reqwest::Error> for Error {
    fn from(value: reqwest::Error) -> Self {
        Self::JupiterApiError(value)
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args = Args::load();

    let (marginfi_account_address, initial_marginfi_account) =
        fetch_marginfi_account(&args.rpc_client, &args.wallet).await?;
    let initial_marginfi_banks = fetch_marginfi_banks(&args.rpc_client).await?;
    let meteora_pools_and_vaults = fetch_meteora_pools_and_vaults(&args.rpc_client).await?;

    let static_addresses = StaticAddresses::new(&args.wallet)
        .set_marginfi_account(marginfi_account_address)
        .set_marginfi_banks(&initial_marginfi_banks)
        .set_meteora_pools_and_vaults(&args.wallet, &meteora_pools_and_vaults)?
        .set_meteora_farms(&args.wallet);

    let websocket_handle = create_persisted_websocket_connection(args.ws_client.clone()).await?;

    let (oracles_state_update_sender, oracles_state_update_receiver) = mpsc::unbounded_channel();
    let oracles_state = Arc::new(OraclesState::new());
    let state_updates_handle =
        OraclesState::listen_to_updates(oracles_state.clone(), oracles_state_update_receiver);

    let pyth_subscription_handle = connection::subscribe_to_pyth_oracles(
        args.ws_client.clone(),
        &static_addresses.marginfi_banks,
        oracles_state_update_sender.clone(),
    );
    let switchboard_subscription_handle = connection::init_and_subscribe_to_switchboard_oracles(
        args.rpc_client.clone(),
        args.ws_client.clone(),
        &static_addresses.marginfi_banks,
        oracles_state_update_sender.clone(),
    )
    .await?;

    let instruction_builder = InstructionBuilder::new(args.wallet.clone());

    sleep(Duration::from_secs(5)).await;

    tokio::select! {
        main_process_res = bot::start(args, initial_marginfi_account, initial_marginfi_banks, oracles_state, static_addresses, instruction_builder) => {
            main_process_res.unwrap()
        }
        websocket_process_res = websocket_handle => {
            websocket_process_res.unwrap().map_err(|e| e.into())
        }
        state_process_res = state_updates_handle => {
            Ok(state_process_res.unwrap())
        }
        pyth_subscription_res = pyth_subscription_handle => {
            pyth_subscription_res.unwrap()
        }
        switchboard_subscription_res = switchboard_subscription_handle => {
            switchboard_subscription_res.unwrap()
        }
    }
}
