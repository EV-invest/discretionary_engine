//! Exchange-agnostic strategy implementation.
//!
//! Strategies in this module only work with Nautilus types (TradeTick, QuoteTick, etc.)
//! and have no knowledge of which exchange the data comes from.

use nautilus_model::data::TradeTick;
use tokio::sync::mpsc;
use tracing::info;

/// A simple strategy that prints trades as they arrive.
///
/// This strategy is completely exchange-agnostic - it only knows about
/// Nautilus `TradeTick` types, not about Bybit, Binance, or any other exchange.
pub struct PrintTradesStrategy {
	trade_rx: mpsc::UnboundedReceiver<TradeTick>,
}

impl PrintTradesStrategy {
	/// Create a new strategy with the given trade receiver.
	pub fn new(trade_rx: mpsc::UnboundedReceiver<TradeTick>) -> Self {
		Self { trade_rx }
	}

	/// Run the strategy, processing trades as they arrive.
	///
	/// This method consumes the strategy and runs until the data stream ends
	/// or an error occurs.
	pub async fn run(mut self) {
		info!("Strategy started, waiting for trades...");

		while let Some(trade) = self.trade_rx.recv().await {
			self.on_trade(&trade);
		}

		info!("Strategy stopped (data stream ended)");
	}

	/// Handle a trade tick - this is where strategy logic would go.
	///
	/// Currently just prints the trade, but this method would contain
	/// actual trading logic in a real strategy.
	fn on_trade(&mut self, trade: &TradeTick) {
		println!(
			"[TRADE] {} | price: {} | size: {} | side: {:?}",
			trade.instrument_id, trade.price, trade.size, trade.aggressor_side
		);
	}
}
