use std::{sync::Arc, time::Duration};

use anchor_lang::prelude::Pubkey;
use fixed::types::I80F48;
use reqwest::Client;
use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::{
    address_lookup_table_account::AddressLookupTableAccount, instruction::Instruction,
};
use solana_transaction_status::UiTransactionStatusMeta;
use tokio::{task::JoinHandle, time::sleep};

use crate::{
    addresses::StaticAddresses,
    args::Args,
    connection, constants,
    instructions::InstructionBuilder,
    state::{MarginfiAccountWithBanks, MarginfiBank, OraclesState},
    utils::transaction::{
        build_signed_transaction, parse_transaction_token_change, send_and_confirm_transaction,
        TransactionResult,
    },
    Error, Wallet,
};

async fn force_send_instructions(
    rpc_client: &Arc<RpcClient>,
    wallet: &Arc<Wallet>,
    instructions: Vec<Instruction>,
    alts: &Vec<AddressLookupTableAccount>,
) -> Result<UiTransactionStatusMeta, Error> {
    let mut tx = build_signed_transaction(rpc_client, wallet, &instructions[..], &alts[..]).await?;
    let mut retries = 0;

    loop {
        if retries % 2 == 0 {
            tx = build_signed_transaction(rpc_client, wallet, &instructions[..], &[]).await?;
        }

        match send_and_confirm_transaction(rpc_client, &tx).await? {
            TransactionResult::Success(sig, meta) => {
                println!("Transaction successful: {}", sig);
                break Ok(meta);
            }
            TransactionResult::Timeout(_) => {}
            TransactionResult::Error(sig, e) => {
                println!("Transaction error: {} - {}", sig, e);
                return Err(Error::TransactionError);
            }
        }

        retries += 1;
    }
}

fn get_best_bank_for_borrow(
    account_with_banks: &MarginfiAccountWithBanks,
) -> (Pubkey, &MarginfiBank) {
    let mut mint_address = Pubkey::default();
    let mut lowest_borrow_rate = I80F48::MAX;
    let mut bank = None;

    for mint in [
        constants::mints::usdc::id(),
        constants::mints::usdt::id(),
        constants::mints::uxd::id(),
    ] {
        let (_, current_bank) = account_with_banks.get_bank_by_mint(&mint).unwrap();
        let borrow_rate = current_bank.get_borrow_rate();

        if borrow_rate < lowest_borrow_rate {
            mint_address = mint;
            lowest_borrow_rate = borrow_rate;
            bank = Some(current_bank);
        }
    }

    (mint_address, bank.unwrap())
}

fn create_marginfi_deposit_instructions(
    account_with_banks: &mut MarginfiAccountWithBanks,
    static_addresses: &StaticAddresses,
    instruction_builder: &InstructionBuilder,
    instructions: &mut Vec<Instruction>,
    bsol_amount: u64,
) -> Result<(), Error> {
    let mint = constants::mints::bsol::id();
    let (_, bank) = account_with_banks.get_bank_by_mint(&mint).unwrap();
    let account_amount = if let Some(balance) = account_with_banks.get_balance_by_mint(&mint) {
        balance
            .get_amounts(bank.asset_share_value, bank.liability_share_value)
            .0
            .to_num()
    } else {
        0
    };

    if account_amount < bsol_amount {
        let deposit_amount =
            bank.get_max_deposit_amount(I80F48::from_num(bsol_amount - account_amount));
        account_with_banks.deposit(deposit_amount, &mint);

        instructions.push(instruction_builder.marginfi_deposit(
            static_addresses,
            &mint,
            deposit_amount.to_num(),
            &account_with_banks,
        )?);
    }

    Ok(())
}

async fn create_marginfi_borrow_instructions(
    account_with_banks: &mut MarginfiAccountWithBanks,
    oracles_state: &Arc<OraclesState>,
    instructions: &mut Vec<Instruction>,
    static_addresses: &StaticAddresses,
    instruction_builder: &InstructionBuilder,
) -> Result<(u64, Pubkey), Error> {
    let (free_amount, _) = account_with_banks
        .get_total_weighted_amount(oracles_state)
        .await?;

    let (mint_to_borrow, bank_for_borrow) = get_best_bank_for_borrow(&account_with_banks);
    // 90% of free amount
    let borrow_amount = free_amount * 9 / 10;
    let borrow_amount_weighted = borrow_amount / bank_for_borrow.liability_weight_init;
    account_with_banks.borrow(borrow_amount_weighted, &mint_to_borrow);

    instructions.push(instruction_builder.marginfi_borrow(
        static_addresses,
        &mint_to_borrow,
        borrow_amount_weighted.to_num(),
        &account_with_banks,
    )?);

    Ok((borrow_amount_weighted.to_num(), mint_to_borrow))
}

pub fn start(
    args: Args,
    initial_marginfi_account: marginfi::state::marginfi_account::MarginfiAccount,
    initial_marginfi_banks: Vec<(Pubkey, marginfi::state::marginfi_group::Bank)>,
    oracles_state: Arc<OraclesState>,
    static_addresses: StaticAddresses,
    instruction_builder: InstructionBuilder,
) -> JoinHandle<Result<(), Error>> {
    tokio::spawn(async move {
        let reqwest_client = Client::new();
        let rpc_client = &args.rpc_client;
        let wallet = &args.wallet;

        let mut account_with_banks =
            MarginfiAccountWithBanks::new(initial_marginfi_account, initial_marginfi_banks);

        {
            let mut instructions = vec![];
            create_marginfi_deposit_instructions(
                &mut account_with_banks,
                &static_addresses,
                &instruction_builder,
                &mut instructions,
                args.bsol_amount,
            )?;
            let (borrowed_amount, borrowed_mint) = create_marginfi_borrow_instructions(
                &mut account_with_banks,
                &oracles_state,
                &mut instructions,
                &static_addresses,
                &instruction_builder,
            )
            .await?;

            force_send_instructions(rpc_client, wallet, instructions, &vec![]).await?;

            let pool_supply_amount = if borrowed_mint != constants::mints::usdc::id() {
                let (swap_ixs, alts) = connection::fetch_swap_instructions(
                    rpc_client,
                    &reqwest_client,
                    wallet,
                    &borrowed_mint,
                    borrowed_amount,
                )
                .await?;
                let tx_meta = force_send_instructions(rpc_client, wallet, swap_ixs, &alts).await?;
                parse_transaction_token_change(
                    &tx_meta,
                    &wallet,
                    &constants::mints::usdc::id(),
                    true,
                )
                .unwrap()
            } else {
                borrowed_amount
            };

            let farm_supply_amount = {
                let meteora_pool =
                    static_addresses.get_meteora_pool(&constants::mints::usdc::id())?;
                let (token_a_amount, token_b_amount) = meteora_pool
                    .get_token_for_deposit(pool_supply_amount, &constants::mints::usdc::id());

                dbg!(pool_supply_amount, token_a_amount, token_b_amount);
                let meteora_deposit_ixs = instruction_builder.meteora_pool_deposit(
                    &static_addresses,
                    meteora_pool,
                    // TODO: Should be based on pool virtual price
                    token_a_amount * 95 / 100,
                    token_a_amount,
                    token_b_amount,
                )?;
                let tx_meta =
                    force_send_instructions(rpc_client, wallet, vec![meteora_deposit_ixs], &vec![])
                        .await?;
                parse_transaction_token_change(&tx_meta, &wallet, &meteora_pool.lp_mint, true)
                    .unwrap()
            };

            {
                let farm_deposit_ix = instruction_builder.meteora_farm_deposit(
                    &static_addresses,
                    &constants::mints::usdc::id(),
                    farm_supply_amount,
                )?;
                force_send_instructions(rpc_client, wallet, vec![farm_deposit_ix], &vec![]).await?;
            }
        }

        loop {
            sleep(Duration::from_secs(60 * 60 * 8)).await;
        }
    })
}
