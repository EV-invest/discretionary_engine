//! Bybit-specific data initialization.
//!
//! This is the ONLY place in the strategy crate that should know about Bybit.
//! All data is normalized to Nautilus types before being sent to the strategy.

use color_eyre::eyre::{Result, bail, eyre};
use futures_util::StreamExt;
use nautilus_bybit::{
	common::enums::{BybitEnvironment, BybitProductType},
	http::client::BybitHttpClient,
	websocket::{client::BybitWebSocketClient, messages::NautilusWsMessage},
};
use nautilus_model::data::{Data, TradeTick};
use smart_default::SmartDefault;
use tokio::sync::mpsc;
use tracing::info;

/// Bybit data source configuration.
#[derive(Clone, Debug, SmartDefault)]
pub struct BybitDataConfig {
	#[default(BybitProductType::Linear)]
	pub product_type: BybitProductType,
	#[default(BybitEnvironment::Mainnet)]
	pub environment: BybitEnvironment,
	#[default(vec!["BTCUSDT".to_string()])]
	pub symbols: Vec<String>,
}
/// Initialize Bybit data stream and return a channel receiver for trades.
///
/// This function handles all Bybit-specific logic:
/// - Fetching instrument data via HTTP
/// - Setting up WebSocket connection
/// - Subscribing to trade feeds
/// - Converting Bybit messages to Nautilus `TradeTick` types
///
/// The returned receiver yields exchange-agnostic `TradeTick` values.
pub async fn init_data(config: BybitDataConfig, tx: mpsc::UnboundedSender<TradeTick>) -> Result<BybitDataHandle> {
	info!("Initializing Bybit data feed for symbols: {:?}", config.symbols);

	// Fetch instrument data via HTTP
	let http_client = BybitHttpClient::new(None, Some(60), None, None, None, None, None)?;

	let mut all_instruments = Vec::default();
	for symbol in &config.symbols {
		let instruments = http_client
			.request_instruments(config.product_type, Some(symbol.clone()))
			.await
			.map_err(|e| eyre!("Failed to fetch instrument {symbol}: {e}"))?;

		if instruments.is_empty() {
			bail!("No instrument found for symbol: {symbol}");
		}
		all_instruments.extend(instruments);
	}

	info!("Fetched {} instrument(s)", all_instruments.len());

	// Create and configure websocket client
	let mut client = BybitWebSocketClient::new_public_with(config.product_type, config.environment, None, None);

	for instrument in all_instruments {
		client.cache_instrument(instrument);
	}

	client.connect().await?;

	// Subscribe to trade feeds
	let subscriptions: Vec<String> = config.symbols.iter().map(|s| format!("publicTrade.{s}")).collect();
	client.subscribe(subscriptions).await?;

	info!("Bybit data feed initialized, streaming trades");

	// Spawn task to forward trades
	let handle = tokio::spawn(async move {
		let stream = client.stream();
		tokio::pin!(stream);

		while let Some(event) = stream.next().await {
			match event {
				NautilusWsMessage::Data(data_vec) => {
					for data in data_vec {
						if let Data::Trade(trade) = data {
							if tx.send(trade).is_err() {
								// Receiver dropped, exit
								break;
							}
						}
					}
				}
				NautilusWsMessage::Error(err) => {
					tracing::error!(code = err.code, message = %err.message, "Bybit websocket error");
				}
				NautilusWsMessage::Reconnected => {
					tracing::warn!("Bybit WebSocket reconnected");
				}
				_ => {}
			}
		}

		let _ = client.close().await;
	});

	Ok(BybitDataHandle { task: handle })
}

/// Handle to the Bybit data streaming task.
#[derive(Debug)]
pub struct BybitDataHandle {
	task: tokio::task::JoinHandle<()>,
}

impl BybitDataHandle {
	/// Wait for the data task to complete.
	pub async fn join(self) -> Result<()> {
		self.task.await.map_err(|e| eyre!("Data task panicked: {e}"))
	}

	/// Abort the data task.
	pub fn abort(&self) {
		self.task.abort();
	}
}
