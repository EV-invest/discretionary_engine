use std::sync::{Arc, RwLock};

use color_eyre::eyre::Result;
use tracing::instrument;

use super::{
	Market,
	binance::BinanceExchange,
	order_types::{ConceptualOrderPercents, ConceptualOrderType, IdRequirements},
};
use crate::{config::LiveSettings, exchange_apis::binance};

/// [Exchange] itself is passed around as Arc<Self>, RwLock is only present at the level of individual exchanges, as to not lock it all at once when writing.
#[derive(Clone, Debug, Default)]
pub struct Exchanges {
	pub binance: Arc<RwLock<BinanceExchange>>,
}
impl Exchanges {
	#[instrument]
	pub async fn init(live_settings: Arc<LiveSettings>) -> Result<Self> {
		let binance = BinanceExchange::init(live_settings.clone()).await?;
		Ok(Self {
			binance: Arc::new(RwLock::new(binance)),
		})
	}

	#[instrument(skip(_s, live_settings))]
	pub async fn compile_total_balance(_s: Arc<Self>, live_settings: Arc<LiveSettings>) -> Result<f64> {
		use exchange_interactions::ExchangeName;
		use secrecy::ExposeSecret;

		let config = live_settings.config()?;
		let binance_config = config.get_exchange(ExchangeName::Binance)?;

		let handlers = vec![
			binance::get_balance(binance_config.api_pubkey.clone(), binance_config.api_secret.expose_secret().to_string(), Market::BinanceFutures),
			binance::get_balance(binance_config.api_pubkey.clone(), binance_config.api_secret.expose_secret().to_string(), Market::BinanceSpot),
		];

		let mut total_balance = 0.0;
		for handler in handlers {
			let balance = handler.await?;
			total_balance += balance;
		}
		Ok(total_balance)
	}

	//TODO!!!!: non-market order's min qty often has another min based on quote_asset, account for that. And also, there often is max percentage-wise diff for how away from the price you can place the order, want to know if we're out of it here.
	/// Returns the absolute minimum trade quantity for (order_type, base_asset) pair
	///
	/// // as min trade qty can depend on whever the order is market or not
	#[instrument(skip(_s))]
	pub fn compile_min_trade_qties(_s: Arc<Self>, base_asset: &str, orders: &[ConceptualOrderPercents]) -> Vec<f64> {
		let ordertypes: Vec<ConceptualOrderType> = orders.iter().map(|o| o.order_type).collect();
		let mut min_notional_qties_accross_exchanges = Vec::with_capacity(ordertypes.len());
		for _ in 0..ordertypes.len() {
			min_notional_qties_accross_exchanges.push(f64::MAX);
		}

		let binance_min_notional_qties = {
			let binance_lock = _s.binance.read().unwrap();
			binance_lock.min_qties_batch(base_asset, &ordertypes)
		};
		assert_eq!(binance_min_notional_qties.len(), ordertypes.len());
		assert_ne!(min_notional_qties_accross_exchanges.len(), 0);
		for (i, q) in binance_min_notional_qties.iter().enumerate() {
			if *q < min_notional_qties_accross_exchanges[i] {
				min_notional_qties_accross_exchanges[i] = *q;
			}
		}

		//- same for other exchanges

		min_notional_qties_accross_exchanges
	}

	/// Value such that any order with notional above it can be executed. Regardless of its type and price.
	///
	/// We find max of the min_qty values for all order_types here, while for limits and stop markets we take the maximum distance from the price exchange allows for.
	#[instrument(skip(_s))]
	pub fn min_qty_any_ordertype(_s: Arc<Self>, base_asset: &str) -> f64 {
		//- same for other exchanges

		{
			let binance_lock = _s.binance.read().unwrap();
			binance_lock.min_qty_any_ordertype(base_asset)
		}
	}
}
