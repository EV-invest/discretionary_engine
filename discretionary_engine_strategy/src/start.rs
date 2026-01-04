//! Start command implementation.
//!
//! This module wires together the exchange-specific data layer with the
//! exchange-agnostic strategy layer.

use color_eyre::eyre::Result;
use tokio::{signal, sync::mpsc};
use tracing::info;

use crate::{
	data::bybit::{BybitDataConfig, init_data},
	strategy::PrintTradesStrategy,
};

/// Start the strategy with trade data feed.
pub async fn start() -> Result<()> {
	// Create channel for trades (the bridge between data layer and strategy)
	let (trade_tx, trade_rx) = mpsc::unbounded_channel();

	// we ONLY talk about specifics in DataFeed initialization
	let config = BybitDataConfig::default();
	let data_handle = init_data(config, trade_tx).await?;

	// Create and run exchange-agnostic strategy
	let strategy = PrintTradesStrategy::new(trade_rx);

	info!("Press Ctrl+C to exit");

	// Run strategy with graceful shutdown
	tokio::select! {
		_ = strategy.run() => {
			info!("Strategy completed");
		}
		_ = signal::ctrl_c() => {
			info!("Received Ctrl+C, shutting down");
			data_handle.abort();
		}
	}

	Ok(())
}
