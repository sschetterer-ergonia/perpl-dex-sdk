//! Example: Print all trades from the testnet exchange.
//!
//! Run with: cargo run --example print_trades

use std::time::Duration;

use alloy::{
    providers::{Provider, ProviderBuilder},
    rpc::client::RpcClient,
    transports::layers::RetryBackoffLayer,
};
use dex_sdk::{fill, types::StateInstant, Chain};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let client = RpcClient::builder()
        .layer(RetryBackoffLayer::new(10, 100, 200))
        .connect("https://testnet-rpc.monad.xyz")
        .await?;
    client.set_poll_interval(Duration::from_millis(500));
    let provider = ProviderBuilder::new().connect_client(client);

    let chain = Chain::testnet();

    // Start from the current block
    let block_num = provider.get_block_number().await?;
    println!("Starting from block {}", block_num);

    let (mut rx, _handle) = fill::start(
        &chain,
        provider,
        StateInstant::new(block_num, 0),
        tokio::time::sleep,
    )
    .await?;

    println!("Listening for trades...\n");

    while let Some(block_trades) = rx.recv().await {
        if !block_trades.is_empty() {
            println!(
                "Block {} - {} trade(s):",
                block_trades.instant.block_number(),
                block_trades.len()
            );
            for trade in &block_trades.trades {
                println!(
                    "  perp={} price={} size={} maker={} taker={} fees=({}, {})",
                    trade.perpetual_id,
                    trade.price,
                    trade.size,
                    trade.maker_account_id,
                    trade.taker_account_id,
                    trade.maker_fee,
                    trade.taker_fee,
                );
            }
        }
    }

    Ok(())
}
