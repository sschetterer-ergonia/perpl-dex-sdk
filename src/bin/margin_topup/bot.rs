//! Margin top-up bot orchestration and event loop.
//!
//! This module contains the bot that monitors positions and automatically
//! tops up collateral when leverage exceeds the configured threshold.

use alloy::{
    network::EthereumWallet,
    primitives::Address,
    providers::{DynProvider, ProviderBuilder},
    rpc::client::RpcClient,
};
use dex_sdk::{
    Chain,
    abi::dex::Exchange::ExchangeInstance,
    state::{Exchange, SnapshotBuilder},
    stream,
    types::{AccountId, OrderRequest, RequestType},
};
use fastnum::{UD64, UD128};
use futures::StreamExt;
use std::{pin::pin, time::Duration};
use tracing::{debug, error, info, warn};
use url::Url;

use crate::{
    error::{Error, Result},
    margin::{self, TopUpAction, TopUpConfig},
};

/// Margin top-up bot.
#[derive(Debug)]
pub struct MarginTopUpBot {
    provider: DynProvider,
    wallet_address: Address,
    instance: ExchangeInstance<DynProvider>,
    chain: Chain,
    config: TopUpConfig,
    timeout: Duration,
    post_tx_delay: Duration,
    account_id: Option<AccountId>,
}

impl MarginTopUpBot {
    /// Create a new margin top-up bot.
    pub async fn try_new(
        node_url: Url,
        wallet: EthereumWallet,
        chain: Chain,
        config: TopUpConfig,
        timeout: Duration,
    ) -> Result<Self> {
        let wallet_address = wallet.default_signer().address();
        info!(
            %wallet_address,
            trigger_leverage = %config.trigger_leverage,
            target_leverage = %config.target_leverage,
            perpetual_ids = ?config.perpetual_ids,
            "Initializing Margin Top-Up Bot"
        );

        let rpc_client = RpcClient::new_http(node_url);
        let provider = DynProvider::new(
            ProviderBuilder::new()
                .wallet(wallet)
                .connect_client(rpc_client),
        );

        let instance = ExchangeInstance::new(chain.exchange(), provider.clone());

        Ok(Self {
            provider,
            wallet_address,
            instance,
            chain,
            config,
            timeout,
            post_tx_delay: Duration::from_secs(2),
            account_id: None,
        })
    }

    /// Run the bot's main event loop.
    pub async fn run(&mut self) -> Result<()> {
        loop {
            info!("Starting new exchange snapshot and event stream");

            // Determine which perpetuals to track
            let perpetual_ids = if self.config.perpetual_ids.is_empty() {
                // If no specific perpetuals, use the chain's configured ones
                self.chain.perpetuals().to_vec()
            } else {
                self.config.perpetual_ids.clone()
            };

            let snapshot_builder = SnapshotBuilder::new(&self.chain, self.provider.clone())
                .with_accounts(vec![self.wallet_address])
                .with_perpetuals(perpetual_ids.clone());

            let mut exchange = snapshot_builder.build().await?;
            info!("Exchange snapshot built successfully");

            // Initialize account ID from snapshot
            self.initialize_account(&exchange)?;

            let instant = exchange.instant();
            let mut dex_stream = pin!(stream::raw(
                &self.chain,
                self.provider.clone(),
                instant,
                tokio::time::sleep,
            ));

            let mut interval = tokio::time::interval(self.timeout);
            interval.tick().await; // First tick completes immediately

            loop {
                tokio::select! {
                    event = dex_stream.next() => {
                        let Some(event) = event else {
                            error!("DEX stream closed unexpectedly, restarting...");
                            break;
                        };

                        let Ok(event) = event else {
                            error!("Error in DEX event stream, will auto-restart");
                            break;
                        };

                        // Apply events to exchange state
                        if let Err(e) = exchange.apply_events(&event) {
                            warn!(?e, "Failed to apply events, continuing...");
                            continue;
                        }

                        // Evaluate positions and potentially top up
                        self.evaluate_and_topup(&exchange).await;
                    }
                    _ = interval.tick() => {
                        // Periodic evaluation even without events
                        debug!("Periodic evaluation triggered");
                        self.evaluate_and_topup(&exchange).await;
                    }
                }
            }
        }
    }

    /// Initialize the account ID from the exchange snapshot.
    fn initialize_account(&mut self, exchange: &Exchange) -> Result<()> {
        let accounts = exchange.accounts();

        if accounts.is_empty() {
            return Err(Error::NoAccountFound);
        }

        if accounts.len() > 1 {
            warn!("Multiple accounts found, using first one");
        }

        let account_id = *accounts.keys().next().unwrap();
        self.account_id = Some(account_id);

        info!(%account_id, "Account initialized");
        Ok(())
    }

    /// Evaluate all positions and execute a top-up if needed.
    async fn evaluate_and_topup(&self, exchange: &Exchange) {
        // Get evaluation summary for logging
        let summary = margin::strategy::evaluate_all(exchange.accounts(), &self.config);

        // Log summary
        if summary.over_leveraged_count > 0 {
            info!(
                positions_evaluated = summary.positions_evaluated,
                over_leveraged = summary.over_leveraged_count,
                can_topup = summary.positions_that_can_topup,
                total_capital_needed = %summary.total_capital_needed,
                available_capital = %summary.available_capital,
                "Position evaluation summary"
            );

            // Log details for over-leveraged positions
            for info in &summary.position_infos {
                if info.is_over_leveraged {
                    if let Some(leverage) = info.current_leverage {
                        if info.can_topup {
                            info!(
                                perpetual_id = %info.perpetual_id,
                                current_leverage = %leverage,
                                required_topup = %info.required_topup.unwrap_or(UD128::ZERO),
                                "Position over-leveraged, top-up available"
                            );
                        } else {
                            error!(
                                perpetual_id = %info.perpetual_id,
                                current_leverage = %leverage,
                                required_topup = %info.required_topup.unwrap_or(UD128::ZERO),
                                available_capital = %summary.available_capital,
                                "INSUFFICIENT CAPITAL: Cannot top up over-leveraged position"
                            );
                        }
                    }
                }
            }
        }

        // Compute the single top-up action (if any)
        let action = margin::strategy::compute_topup(exchange.accounts(), &self.config);

        if let Some(action) = action {
            info!(
                perpetual_id = %action.perpetual_id,
                amount = %action.amount,
                current_leverage = %action.current_leverage,
                target_leverage = %action.target_leverage,
                "Executing top-up"
            );

            if let Err(e) = self.execute_topup(exchange, &action).await {
                error!(?e, "Failed to execute top-up");
            } else {
                info!(
                    perpetual_id = %action.perpetual_id,
                    amount = %action.amount,
                    "Top-up transaction submitted successfully"
                );

                // Wait for event stream to catch up
                tokio::time::sleep(self.post_tx_delay).await;
            }
        }
    }

    /// Execute a single top-up transaction.
    async fn execute_topup(&self, exchange: &Exchange, action: &TopUpAction) -> Result<()> {
        let request = OrderRequest::new(
            0, // request_id - not used for IncreasePositionCollateral
            action.perpetual_id,
            RequestType::IncreasePositionCollateral,
            None,           // order_id - not used
            UD64::ZERO,     // price - not used
            UD64::ZERO,     // size - not used
            None,           // expiry_block - not used
            false,          // post_only - not used
            false,          // fill_or_kill - not used
            false,          // immediate_or_cancel - not used
            None,           // max_matches - not used
            UD64::ONE,      // leverage - not used
            None,           // last_exec_block - not used
            Some(action.amount), // collateral to add
        );

        let order_desc = request.prepare(exchange);

        debug!(?order_desc, "Prepared IncreasePositionCollateral order");

        let builder = self
            .instance
            .execOpsAndOrders(vec![], vec![order_desc], false);

        let pending_tx = builder.send().await?;
        let receipt = pending_tx.get_receipt().await?;

        debug!(?receipt, "Top-up transaction receipt");

        if !receipt.status() {
            error!("Top-up transaction failed (reverted)");
        }

        Ok(())
    }
}
