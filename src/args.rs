use std::{str::FromStr, sync::Arc};

use anchor_lang::prelude::Pubkey;
use clap::Parser;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{commitment_config::CommitmentConfig, signature::Keypair, signer::Signer};

use crate::{utils::websocket_client::WebsocketClient, Wallet};

const NAMESPACE: &'static str = "[CONFIG_ERROR]:";

pub fn load_arg(key: &str) -> String {
    std::env::var(key).expect(&format!("{NAMESPACE} Argument {key} is missing"))
}

pub fn load_and_parse_arg<T, F: Fn(String) -> Result<T, String>>(key: &str, parse_fn: F) -> T {
    parse_fn(load_arg(key)).expect(&format!("{NAMESPACE} Could not parse {key} argument"))
}

#[derive(Debug, Parser)]
pub struct CliArgs {
    #[arg(long = "bsol", default_value_t = 0.0)]
    bsol_amount: f32,

    #[arg(long, default_value_t = false)]
    update_alt: bool,
}

pub struct Args {
    pub bsol_amount: u64,
    pub rpc_client: Arc<RpcClient>,
    pub ws_client: Arc<WebsocketClient>,
    pub wallet: Arc<Wallet>,
    pub alt_address: Pubkey,
}

impl Args {
    pub fn load() -> Self {
        dotenv::dotenv().ok();

        let rpc_client = load_and_parse_arg("RPC_URL", |url| {
            Ok(Arc::new(RpcClient::new_with_commitment(
                url,
                CommitmentConfig::confirmed(),
            )))
        });
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
        let alt_address = load_and_parse_arg("ADDRESS_LOOKUP_TABLE", |alt| {
            Ok(Pubkey::from_str(&alt).map_err(|_| "Invalid ALT address")?)
        });

        let cli_args = CliArgs::parse();
        let bsol_amount = (cli_args.bsol_amount * 10_f32.powf(9.0)) as u64;

        Self {
            bsol_amount,
            rpc_client,
            ws_client,
            wallet,
            alt_address,
        }
    }
}
