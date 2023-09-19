use std::sync::Arc;

use anchor_lang::prelude::Pubkey;
use fixed::types::I80F48;
use marginfi::{
    constants::{CONF_INTERVAL_MULTIPLE, EXP_10, EXP_10_I80F48},
    state::{marginfi_account::Balance, marginfi_group::Bank as OnChainBank, price::OracleSetup},
};
use switchboard_v2::{AggregatorAccountData, AggregatorResolutionMode, SwitchboardDecimal};
use tokio::{
    sync::{mpsc, Mutex},
    task::JoinHandle,
};

use crate::Error;

#[inline]
fn pyth_price_components_to_i80f48(price: I80F48, exponent: i32) -> Result<I80F48, Error> {
    let scaling_factor = EXP_10_I80F48[exponent.unsigned_abs() as usize];

    if exponent == 0 {
        Ok(price)
    } else if exponent < 0 {
        price
            .checked_div(scaling_factor)
            .ok_or(Error::UnableToParsePythOracle)
    } else {
        price
            .checked_mul(scaling_factor)
            .ok_or(Error::UnableToParsePythOracle)
    }
}

#[inline]
fn fit_scale_switchboard_decimal(
    decimal: SwitchboardDecimal,
    scale: u32,
) -> Option<SwitchboardDecimal> {
    if decimal.scale <= scale {
        return Some(decimal);
    }

    let scale_diff = decimal.scale - scale;
    let mantissa = decimal.mantissa.checked_div(EXP_10[scale_diff as usize])?;

    Some(SwitchboardDecimal { mantissa, scale })
}

#[inline(always)]
fn swithcboard_decimal_to_i80f48(decimal: SwitchboardDecimal) -> Option<I80F48> {
    const MAX_SCALE: u32 = 20;

    let decimal = fit_scale_switchboard_decimal(decimal, MAX_SCALE)?;
    I80F48::from_num(decimal.mantissa).checked_div(EXP_10_I80F48[decimal.scale as usize])
}

pub trait PriceData {
    fn get_price(&self) -> Result<I80F48, Error>;

    fn get_confidence_interval(&self) -> Result<I80F48, Error>;

    fn get_price_range(&self) -> Result<(I80F48, I80F48), Error>;
}

#[derive(Clone, Debug)]
pub struct PythPriceFeed {
    pub last_update_slot: u64,
    pub price: pyth_sdk_solana::Price,
}

impl PriceData for PythPriceFeed {
    fn get_price(&self) -> Result<I80F48, Error> {
        pyth_price_components_to_i80f48(I80F48::from_num(self.price.price), self.price.expo)
    }

    fn get_confidence_interval(&self) -> Result<I80F48, Error> {
        let conf_interval =
            pyth_price_components_to_i80f48(I80F48::from_num(self.price.conf), self.price.expo)?
                .checked_mul(CONF_INTERVAL_MULTIPLE)
                .ok_or(Error::UnableToParsePythOracle)?;

        // assert!(
        //     conf_interval >= I80F48::ZERO,
        //     "Negative confidence interval"
        // );

        Ok(conf_interval)
    }

    fn get_price_range(&self) -> Result<(I80F48, I80F48), Error> {
        let base_price = self.get_price()?;
        let price_range = self.get_confidence_interval()?;

        let lowest_price = base_price
            .checked_sub(price_range)
            .ok_or(Error::UnableToParsePythOracle)?;
        let highest_price = base_price
            .checked_add(price_range)
            .ok_or(Error::UnableToParsePythOracle)?;

        Ok((lowest_price, highest_price))
    }
}

#[derive(Clone, Debug)]
pub struct SwitchboardPriceFeed {
    pub last_update_ts: i64,
    pub resolution_mode: AggregatorResolutionMode,
    pub latest_confirmed_round_result: SwitchboardDecimal,
    pub latest_confirmed_round_num_success: u32,
    pub latest_confirmed_round_std_deviation: SwitchboardDecimal,
    pub min_oracle_results: u32,
}

impl From<&AggregatorAccountData> for SwitchboardPriceFeed {
    fn from(agg: &AggregatorAccountData) -> Self {
        Self {
            last_update_ts: agg.latest_confirmed_round.round_open_timestamp,
            resolution_mode: agg.resolution_mode,
            latest_confirmed_round_result: agg.latest_confirmed_round.result,
            latest_confirmed_round_num_success: agg.latest_confirmed_round.num_success,
            latest_confirmed_round_std_deviation: agg.latest_confirmed_round.std_deviation,
            min_oracle_results: agg.min_oracle_results,
        }
    }
}

impl SwitchboardPriceFeed {
    fn get_result(&self) -> Result<SwitchboardDecimal, Error> {
        if self.resolution_mode == AggregatorResolutionMode::ModeSlidingResolution {
            return Ok(self.latest_confirmed_round_result);
        }
        let min_oracle_results = self.min_oracle_results;
        let latest_confirmed_round_num_success = self.latest_confirmed_round_num_success;
        if min_oracle_results > latest_confirmed_round_num_success {
            return Err(Error::UnableToParseSwitchboardOracle);
        }
        Ok(self.latest_confirmed_round_result)
    }
}

impl PriceData for SwitchboardPriceFeed {
    fn get_price(&self) -> Result<I80F48, Error> {
        let sw_decimal = self
            .get_result()
            .map_err(|_| Error::UnableToParseSwitchboardOracle)?;

        Ok(swithcboard_decimal_to_i80f48(sw_decimal)
            .ok_or(Error::UnableToParseSwitchboardOracle)?)
    }

    fn get_confidence_interval(&self) -> Result<I80F48, Error> {
        let std_div = self.latest_confirmed_round_std_deviation;
        let std_div =
            swithcboard_decimal_to_i80f48(std_div).ok_or(Error::UnableToParseSwitchboardOracle)?;

        let conf_interval = std_div
            .checked_mul(CONF_INTERVAL_MULTIPLE)
            .ok_or(Error::UnableToParseSwitchboardOracle)?;

        // assert!(
        //     conf_interval >= I80F48::ZERO,
        //     "Negative confidence interval"
        // );

        Ok(conf_interval)
    }

    fn get_price_range(&self) -> Result<(I80F48, I80F48), Error> {
        let base_price = self.get_price()?;
        let price_range = self.get_confidence_interval()?;

        let lowest_price = base_price
            .checked_sub(price_range)
            .ok_or(Error::UnableToParseSwitchboardOracle)?;
        let highest_price = base_price
            .checked_add(price_range)
            .ok_or(Error::UnableToParseSwitchboardOracle)?;

        Ok((lowest_price, highest_price))
    }
}

pub enum StateUpdate {
    PythOracle((Pubkey, PythPriceFeed)),
    SwitchboardOracle((Pubkey, SwitchboardPriceFeed)),
}

#[derive(Debug)]
pub struct OraclesState {
    pub pyth_oracles: Mutex<Vec<(Pubkey, PythPriceFeed)>>,
    pub switchboard_oracles: Mutex<Vec<(Pubkey, SwitchboardPriceFeed)>>,
}

impl OraclesState {
    pub fn new() -> Self {
        Self {
            pyth_oracles: Default::default(),
            switchboard_oracles: Default::default(),
        }
    }

    pub async fn get_oracle(
        &self,
        oracle_type: OracleSetup,
        oracle_address: &Pubkey,
    ) -> Option<Box<dyn PriceData>> {
        match oracle_type {
            OracleSetup::PythEma => {
                let pyth_oracles = self.pyth_oracles.lock().await;

                pyth_oracles
                    .iter()
                    .find(|(address, _)| address == oracle_address)
                    .cloned()
                    .map(|(_, p)| Box::new(p) as Box<dyn PriceData>)
            }
            OracleSetup::SwitchboardV2 => {
                let switchboard_oracles = self.switchboard_oracles.lock().await;

                switchboard_oracles
                    .iter()
                    .find(|(address, _)| address == oracle_address)
                    .cloned()
                    .map(|(_, p)| Box::new(p) as Box<dyn PriceData>)
            }
            OracleSetup::None => unreachable!(),
        }
    }

    pub fn listen_to_updates(
        state: Arc<Self>,
        mut update_receiver: mpsc::UnboundedReceiver<StateUpdate>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(update) = update_receiver.recv().await {
                match update {
                    StateUpdate::PythOracle((address, price_feed)) => {
                        let mut oracles = state.pyth_oracles.lock().await;

                        if let Some(saved_oracle) =
                            oracles.iter_mut().find(|(addr, _)| addr == &address)
                        {
                            saved_oracle.1 = price_feed;
                        } else {
                            oracles.push((address, price_feed));
                        }
                    }
                    StateUpdate::SwitchboardOracle((address, price_feed)) => {
                        let mut oracles = state.switchboard_oracles.lock().await;

                        if let Some(saved_oracle) =
                            oracles.iter_mut().find(|(addr, _)| addr == &address)
                        {
                            saved_oracle.1 = price_feed;
                        } else {
                            oracles.push((address, price_feed));
                        }
                    }
                }
            }
        })
    }
}

fn calc_scaled_amount(
    amount: I80F48,
    weight: Option<I80F48>,
    price: I80F48,
    scaling_factor: I80F48,
) -> I80F48 {
    let weighted = if let Some(w) = weight {
        amount * w
    } else {
        amount
    };
    weighted * price / scaling_factor
}

#[derive(Debug)]
pub struct MarginfiBank {
    pub mint: Pubkey,
    pub mint_decimals: u8,
    pub total_asset_value_init_limit: u64,
    pub oracle_setup: OracleSetup,
    pub oracle_address: Pubkey,

    pub asset_weight_init: I80F48,
    pub liability_weight_init: I80F48,

    pub asset_share_value: I80F48,
    pub liability_share_value: I80F48,

    pub total_asset_shares: I80F48,
    pub total_liability_shares: I80F48,

    pub optimal_utilization_rate: I80F48,
    pub plateau_interest_rate: I80F48,
    pub max_interest_rate: I80F48,
}

impl Default for MarginfiBank {
    fn default() -> Self {
        Self {
            oracle_setup: OracleSetup::PythEma,
            mint: Default::default(),
            mint_decimals: Default::default(),
            total_asset_value_init_limit: Default::default(),
            oracle_address: Default::default(),

            asset_weight_init: Default::default(),
            liability_weight_init: Default::default(),

            asset_share_value: Default::default(),
            liability_share_value: Default::default(),

            total_asset_shares: Default::default(),
            total_liability_shares: Default::default(),

            optimal_utilization_rate: Default::default(),
            plateau_interest_rate: Default::default(),
            max_interest_rate: Default::default(),
        }
    }
}

impl From<marginfi::state::marginfi_group::Bank> for MarginfiBank {
    fn from(bank: marginfi::state::marginfi_group::Bank) -> Self {
        Self {
            mint: bank.mint,
            mint_decimals: bank.mint_decimals,
            total_asset_value_init_limit: bank.config.total_asset_value_init_limit,
            oracle_setup: bank.config.oracle_setup,
            oracle_address: bank.config.oracle_keys[0],
            asset_weight_init: I80F48::from_bits(bank.config.asset_weight_init.value),
            liability_weight_init: I80F48::from_bits(bank.config.liability_weight_init.value),
            asset_share_value: I80F48::from_bits(bank.asset_share_value.value),
            liability_share_value: I80F48::from_bits(bank.liability_share_value.value),
            total_asset_shares: I80F48::from_bits(bank.total_asset_shares.value),
            total_liability_shares: I80F48::from_bits(bank.total_liability_shares.value),
            optimal_utilization_rate: I80F48::from_bits(
                bank.config
                    .interest_rate_config
                    .optimal_utilization_rate
                    .value,
            ),
            plateau_interest_rate: I80F48::from_bits(
                bank.config.interest_rate_config.plateau_interest_rate.value,
            ),
            max_interest_rate: I80F48::from_bits(
                bank.config.interest_rate_config.max_interest_rate.value,
            ),
        }
    }
}

impl MarginfiBank {
    pub fn get_max_deposit_amount(&self, deposit_amount: I80F48) -> I80F48 {
        let mut max_deposit_amount = I80F48::from_num(self.total_asset_value_init_limit);

        if max_deposit_amount == 0 {
            return deposit_amount;
        } else {
            max_deposit_amount = max_deposit_amount * EXP_10_I80F48[self.mint_decimals as usize];
        }

        let total_deposit_amount = self.asset_share_value * self.total_asset_shares;

        if max_deposit_amount <= total_deposit_amount {
            return I80F48::ZERO;
        }

        deposit_amount.min(max_deposit_amount - total_deposit_amount)
    }

    pub fn get_borrow_rate(&self) -> I80F48 {
        if self.total_liability_shares == 0 {
            return I80F48::ZERO;
        }

        let current_utilization = self.total_liability_shares / self.total_asset_shares;

        if current_utilization <= self.optimal_utilization_rate {
            current_utilization / self.optimal_utilization_rate * self.plateau_interest_rate
        } else {
            let u = current_utilization - self.optimal_utilization_rate;
            let l = I80F48::ONE - self.optimal_utilization_rate;
            (u / l) * (self.max_interest_rate - self.plateau_interest_rate)
                + self.plateau_interest_rate
        }
    }
}

#[derive(Debug, Default)]
pub struct MarginfiAccountBalance {
    pub is_active: bool,
    pub bank_address: Pubkey,
    pub asset_shares: I80F48,
    pub liability_shares: I80F48,
    pub asset_weight: I80F48,
    pub liabilities_weight: I80F48,
}

impl MarginfiAccountBalance {
    pub fn new(balance: &Balance, bank: &MarginfiBank) -> Self {
        let asset_shares = I80F48::from_bits(balance.asset_shares.value);
        let liabilities_shares = I80F48::from_bits(balance.liability_shares.value);

        Self {
            is_active: balance.active,
            asset_shares,
            liability_shares: liabilities_shares,
            bank_address: balance.bank_pk,
            asset_weight: bank.asset_weight_init,
            liabilities_weight: bank.liability_weight_init,
        }
    }

    pub fn new_empty(bank_address: &Pubkey, bank: &MarginfiBank) -> Self {
        Self {
            bank_address: *bank_address,
            asset_shares: I80F48::ZERO,
            liability_shares: I80F48::ZERO,
            is_active: false,
            asset_weight: bank.asset_weight_init,
            liabilities_weight: bank.liability_weight_init,
        }
    }

    pub fn get_amounts(
        &self,
        asset_share_value: I80F48,
        liab_share_value: I80F48,
    ) -> (I80F48, I80F48) {
        (
            self.asset_shares * asset_share_value,
            self.liability_shares * liab_share_value,
        )
    }

    pub fn get_weighted_amounts(
        &self,
        bank: &MarginfiBank,
        oracle: &Box<dyn PriceData>,
    ) -> Result<(I80F48, I80F48), Error> {
        if !self.is_active {
            return Ok((I80F48::ZERO, I80F48::ZERO));
        }

        let asset_share_value = bank.asset_share_value;
        let liability_share_value = bank.liability_share_value;

        let (worst_price, best_price) = oracle.get_price_range()?;
        let (asset_amount, liab_amount) =
            self.get_amounts(asset_share_value, liability_share_value);

        let scaling_factor = EXP_10_I80F48[bank.mint_decimals as usize];
        let mut total_assets = calc_scaled_amount(
            asset_amount,
            Some(self.asset_weight),
            worst_price,
            scaling_factor,
        );
        let total_liabilities = calc_scaled_amount(
            liab_amount,
            Some(self.liabilities_weight),
            best_price,
            scaling_factor,
        );

        if bank.total_asset_value_init_limit != 0 {
            let bank_total_assets = calc_scaled_amount(
                bank.total_asset_shares * asset_share_value,
                None,
                worst_price,
                scaling_factor,
            );
            let total_asset_value_init_limit = I80F48::from_num(bank.total_asset_value_init_limit);

            if bank_total_assets > total_asset_value_init_limit {
                let discount = total_asset_value_init_limit / bank_total_assets;
                dbg!(discount);
                total_assets = total_assets * discount;
            }
        }

        Ok((
            total_assets * EXP_10_I80F48[6],
            total_liabilities * EXP_10_I80F48[6],
        ))
    }
}

#[derive(Debug, Default)]
pub struct MarginfiAccountWithBanks {
    pub balances: Vec<(Pubkey, MarginfiAccountBalance)>,
    pub banks: Vec<(Pubkey, MarginfiBank)>,
}

impl MarginfiAccountWithBanks {
    pub fn new(
        on_chain_account: marginfi::state::marginfi_account::MarginfiAccount,
        on_chain_banks: Vec<(Pubkey, OnChainBank)>,
    ) -> Self {
        let mut acc = Self::default();
        acc.update_banks(on_chain_banks);
        acc.update_balances(on_chain_account);
        acc
    }

    pub fn update_banks(&mut self, on_chain_banks: Vec<(Pubkey, OnChainBank)>) {
        for (bank_address, bank) in on_chain_banks {
            let b = MarginfiBank::from(bank);
            self.banks.push((bank_address, b))
        }
    }

    pub fn update_balances(
        &mut self,
        on_chain_account: marginfi::state::marginfi_account::MarginfiAccount,
    ) {
        self.balances = vec![];

        for balance in on_chain_account.lending_account.balances.iter() {
            if let Some(bank) = self.get_bank_by_address(&balance.bank_pk) {
                self.balances
                    .push((bank.mint, MarginfiAccountBalance::new(balance, bank)))
            }
        }
    }

    pub fn deposit(&mut self, amount: I80F48, mint: &Pubkey) {
        let (bank_address, bank) = &self.get_bank_by_mint(mint).unwrap();
        let asset_shares = amount / bank.asset_share_value;

        if let Some(i) = self.balances.iter().position(|(m, _)| m == mint) {
            let (_, balance) = &mut self.balances[i];
            balance.asset_shares = balance.asset_shares + asset_shares;
        } else {
            let mut balance = MarginfiAccountBalance::new_empty(bank_address, bank);
            balance.is_active = true;
            balance.asset_shares = asset_shares;

            self.balances.push((*mint, balance));
        }
    }

    pub fn borrow(&mut self, amount: I80F48, mint: &Pubkey) {
        let (bank_address, bank) = &self.get_bank_by_mint(mint).unwrap();
        let liability_shares = amount / bank.liability_share_value;

        if let Some(i) = self.balances.iter().position(|(m, _)| m == mint) {
            let (_, balance) = &mut self.balances[i];
            balance.asset_shares = balance.liability_shares + liability_shares;
        } else {
            let mut balance = MarginfiAccountBalance::new_empty(bank_address, bank);
            balance.is_active = true;
            balance.liabilities_weight = liability_shares;

            self.balances.push((*mint, balance));
        }
    }

    pub fn get_bank_by_mint(&self, mint: &Pubkey) -> Option<&(Pubkey, MarginfiBank)> {
        self.banks.iter().find(|(_, bank)| &bank.mint == mint)
    }

    pub fn get_bank_by_address(&self, address: &Pubkey) -> Option<&MarginfiBank> {
        self.banks
            .iter()
            .find(|(addr, _)| addr == address)
            .map(|(_, bank)| bank)
    }

    pub fn get_balance_by_mint(&self, mint: &Pubkey) -> Option<&MarginfiAccountBalance> {
        self.balances
            .iter()
            .find(|(m, _)| m == mint)
            .map(|(_, b)| b)
    }

    pub async fn get_total_weighted_amount(
        &self,
        oracles_state: &Arc<OraclesState>,
    ) -> Result<(I80F48, I80F48), Error> {
        let mut total_assets = I80F48::ZERO;
        let mut total_liabilities = I80F48::ZERO;

        for (mint, balance) in self.balances.iter() {
            let (_, bank) = self.get_bank_by_mint(mint).unwrap();
            let oracle = oracles_state
                .get_oracle(bank.oracle_setup, &bank.oracle_address)
                .await
                .unwrap();

            let (assets, liabilities) = balance.get_weighted_amounts(bank, &oracle)?;

            total_assets = total_assets + assets;
            total_liabilities = total_liabilities * liabilities;
        }

        Ok((total_assets, total_liabilities))
    }
}
