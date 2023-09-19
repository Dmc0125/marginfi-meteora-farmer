use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anchor_lang::prelude::Pubkey;
use solana_client::{
    client_error::{ClientError, ClientErrorKind},
    nonblocking::rpc_client::RpcClient,
    rpc_config::{RpcSendTransactionConfig, RpcTransactionConfig},
};
use solana_sdk::{
    address_lookup_table_account::AddressLookupTableAccount,
    commitment_config::CommitmentConfig,
    instruction::Instruction,
    message::{v0::Message, VersionedMessage},
    signature::Signature,
    transaction::{TransactionError, VersionedTransaction},
};
use solana_transaction_status::{
    option_serializer::OptionSerializer, UiTransactionEncoding, UiTransactionStatusMeta,
    UiTransactionTokenBalance,
};
use tokio::time::sleep;

use crate::{Error, Wallet};

pub fn parse_transaction_token_change(
    meta: &UiTransactionStatusMeta,
    wallet: &Arc<Wallet>,
    mint: &Pubkey,
    is_positive: bool,
) -> Option<u64> {
    match (&meta.pre_token_balances, &meta.post_token_balances) {
        (
            OptionSerializer::Some(pre_token_balances),
            OptionSerializer::Some(post_token_balances),
        ) => {
            let wallet_str = wallet.pubkey.to_string();
            let mint_str = mint.to_string();

            let is_correct_token_balance = |b: &UiTransactionTokenBalance| {
                if &b.mint != &mint_str {
                    return false;
                }
                match &b.owner {
                    OptionSerializer::Some(owner) => owner == &wallet_str,
                    _ => false,
                }
            };

            match (
                pre_token_balances
                    .iter()
                    .find(|b| is_correct_token_balance(b)),
                post_token_balances
                    .iter()
                    .find(|b| is_correct_token_balance(b)),
            ) {
                (Some(pre_balance), Some(post_balance)) => {
                    let pre_token_amount: u64 = pre_balance.ui_token_amount.amount.parse().unwrap();
                    let post_token_amount: u64 =
                        post_balance.ui_token_amount.amount.parse().unwrap();

                    if is_positive {
                        Some(post_token_amount - pre_token_amount)
                    } else {
                        Some(pre_token_amount - post_token_amount)
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

#[derive(Debug)]
pub enum ClientTransactionError {
    UnableToCompile,
    MissingSigner,
    MissingSignature,
    RpcError,
}

impl From<ClientError> for ClientTransactionError {
    fn from(_: ClientError) -> Self {
        Self::RpcError
    }
}

pub async fn build_signed_transaction(
    rpc_client: &Arc<RpcClient>,
    signer: &Arc<Wallet>,
    instructions: &[Instruction],
    address_lookup_tables: &[AddressLookupTableAccount],
) -> Result<VersionedTransaction, ClientTransactionError> {
    let blockhash = rpc_client.get_latest_blockhash().await?;
    let message = Message::try_compile(
        &signer.pubkey,
        instructions,
        address_lookup_tables,
        blockhash,
    )
    .map_err(|_| ClientTransactionError::UnableToCompile)?;

    let tx = VersionedTransaction::try_new(VersionedMessage::V0(message), &[&signer.keypair])
        .map_err(|_| ClientTransactionError::MissingSigner)?;

    tx.sanitize(true)
        .map_err(|_| ClientTransactionError::MissingSignature)?;

    Ok(tx)
}

const POLL_TIMEOUT: Duration = Duration::from_secs(2);
const TX_VALIDITY_DURATION: u64 = 40;

pub enum TransactionResult {
    Success(Signature, UiTransactionStatusMeta),
    Error(Signature, TransactionError),
    Timeout(Signature),
}

pub async fn send_and_confirm_transaction(
    rpc_client: &Arc<RpcClient>,
    tx: &VersionedTransaction,
) -> Result<TransactionResult, Error> {
    let signature = rpc_client
        .send_transaction_with_config(
            tx,
            RpcSendTransactionConfig {
                skip_preflight: true,
                max_retries: Some(20),
                ..Default::default()
            },
        )
        .await?;
    println!("Sent transaction: {}", signature);
    let start = Instant::now();

    loop {
        if start.elapsed().as_secs() > TX_VALIDITY_DURATION {
            break Ok(TransactionResult::Timeout(signature));
        }

        sleep(POLL_TIMEOUT).await;
        let res = rpc_client
            .get_transaction_with_config(
                &signature,
                RpcTransactionConfig {
                    encoding: Some(UiTransactionEncoding::Base64),
                    commitment: Some(CommitmentConfig::confirmed()),
                    max_supported_transaction_version: Some(0),
                },
            )
            .await;

        match res {
            Err(e) => match e.kind {
                ClientErrorKind::SerdeJson(_) => {}
                _ => Err(e)?,
            },
            Ok(res) => {
                let meta = res.transaction.meta.ok_or(Error::TransactionError)?;

                if let Some(e) = meta.err {
                    return Ok(TransactionResult::Error(signature, e));
                } else {
                    return Ok(TransactionResult::Success(signature, meta));
                }
            }
        }
    }
}
