use std::sync::Arc;

use anchor_lang::{prelude::Pubkey, AccountDeserialize};
use clap::Parser;
use connection::{fetch_marginfi_account, fetch_marginfi_banks};
use solana_account_decoder::UiAccountEncoding;
use solana_client::{
    client_error::ClientError,
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcAccountInfoConfig, RpcProgramAccountsConfig},
    rpc_filter::{Memcmp, RpcFilterType},
};
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair, signer::Signer};
use websocket::WebsocketError;

use crate::{
    args::{load_and_parse_arg, CliArgs},
    websocket::{create_persisted_websocket_connection, WebsocketClient},
};

pub mod args;
pub mod connection;
pub mod websocket;

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

#[tokio::main]
async fn main() -> Result<(), Error> {
    dotenv::dotenv().ok();

    let cli_args = CliArgs::parse();

    let rpc_client = load_and_parse_arg("RPC_URL", |url| Ok(Arc::new(RpcClient::new(url))));
    let ws_client = load_and_parse_arg("WS_URL", |url| Ok(Arc::new(WebsocketClient::new(url))));
    let wallet = load_and_parse_arg("PRIVATE_KEY", |pk| {
        let pk = pk
            .split(",")
            .map(|x| x.parse().map_err(|_| "Invalid private key"))
            .collect::<Result<Vec<u8>, &str>>()?;
        let keypair = Keypair::from_bytes(&pk[..]).map_err(|e| e.to_string())?;
        let pubkey = keypair.try_pubkey().unwrap();
        Ok(Arc::new(Wallet { keypair, pubkey }))
    });

    let handle = create_persisted_websocket_connection(ws_client.clone()).await?;

    Ok(())
}
