use std::{
	collections::HashMap,
	sync::{Arc, RwLock},
};

use ahash::AHashMap;
use serde::{Deserialize, Serialize};
use tracing::instrument;

use super::BinanceExchange;
use crate::{
	exchange_apis::order_types::{Order, OrderType, StopMarketOrder},
	positions::PositionOrderId,
};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct BinanceOrder {
	pub base_info: Order<PositionOrderId>,
	pub binance_id: Option<i64>,
	pub notional_filled: f64,
}
impl BinanceOrder {
	pub fn new(base_info: Order<PositionOrderId>) -> Self {
		Self { base_info, ..Default::default() }
	}

	pub fn to_params(&self) -> AHashMap<&'static str, String> {
		let mut params = AHashMap::<&'static str, String>::default();
		params.insert("symbol", self.base_info.symbol.to_string());
		params.insert("side", self.base_info.side.to_string());
		params.insert("quantity", format!("{}", self.base_info.qty_notional));

		let type_params = match &self.base_info.order_type {
			OrderType::Market => {
				let mut params = AHashMap::<&'static str, String>::default();
				params.insert("type", "MARKET".to_string());
				params
			}
			OrderType::StopMarket(sm) => {
				let mut params = AHashMap::<&'static str, String>::default();
				params.insert("type", "STOP_MARKET".to_string());
				params.insert("stopPrice", sm.price.to_string());
				params
			}
		};
		params.extend(type_params);

		params
	}

	#[instrument(skip(binance_exchange_arc))]
	pub async fn from_standard(mut order: Order<PositionOrderId>, binance_exchange_arc: Arc<RwLock<BinanceExchange>>) -> Self {
		let futures_symbol = {
			let lock = binance_exchange_arc.read().unwrap();
			let futures_symbol = lock
				.binance_futures_info
				.pair(&order.symbol.base, &order.symbol.quote)
				.expect("coin should have been checked earlier");
			futures_symbol.clone()
		};
		fn precision(qty: f64, precision: i32) -> f64 {
			let factor = 10_f64.powi(precision);
			(qty * factor).round() / factor
		}
		let coin_quantity_adjusted = precision(order.qty_notional, futures_symbol.quantity_precision as i32);
		order.qty_notional = coin_quantity_adjusted;

		let order_type = match &order.order_type {
			OrderType::Market => OrderType::Market,
			OrderType::StopMarket(sm) => OrderType::StopMarket(StopMarketOrder::new(precision(sm.price, futures_symbol.price_precision as i32))),
		};
		order.order_type = order_type;

		Self::new(order)
	}
}
