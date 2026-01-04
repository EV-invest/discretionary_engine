#[allow(unused_imports)] // RA bug
use std::str::FromStr;

use color_eyre::eyre::Result;
use discretionary_engine_macros::ProtocolWrapper;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::{sync::mpsc, task::JoinSet};
use tokio_tungstenite::connect_async;
use tracing::{debug, instrument};
use v_utils::{
	Percent,
	macros::CompactFormat,
	trades::{Ohlc, Side, Timeframe},
};

use crate::{
	exchange_apis::{Market, Symbol, order_types::*},
	protocols::{ProtocolOrders, ProtocolTrait, ProtocolType},
};

const BINANCE_TIMEFRAMES: [&str; 19] = [
	"1s", "5s", "15s", "30s", "1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "6h", "8h", "12h", "1d", "3d", "1w", "1M",
];

#[derive(Clone, CompactFormat, Copy, Debug, Default, ProtocolWrapper, derive_new::new)]
pub struct Sar {
	start: Percent,
	increment: Percent,
	max: Percent,
	/// NB: Impossible to change dynamically currently (because the websocket connection contains timeframe in its signature)
	timeframe: Timeframe,
}

impl ProtocolTrait for SarWrapper {
	type Params = Sar;

	#[instrument(skip(position_js, tx_orders))]
	fn attach(&self, position_js: &mut JoinSet<Result<()>>, tx_orders: mpsc::Sender<ProtocolOrders>, asset: String, protocol_side: Side) -> Result<()> {
		let symbol = Symbol {
			base: asset,
			quote: "USDT".to_owned(),
			market: Market::BinanceFutures,
		};
		let tf = { self.0.read().unwrap().timeframe };
		let (tx, mut rx) = tokio::sync::mpsc::channel::<Ohlc>(256);
		let mut last_order: Option<ConceptualOrderPercents> = None;
		let symbol_clone = symbol.clone();
		let params_arc = self.0.clone();
		position_js.spawn(async move {
			let mut js = JoinSet::new();
			js.spawn(async move {
				let address = format!("wss://fstream.binance.com/ws/{}@kline_{tf}", symbol_clone.to_string().to_lowercase());
				let (ws_stream, _) = connect_async(address).await.unwrap();
				let (_, mut read) = ws_stream.split();

				while let Some(msg) = read.next().await {
					let data = msg.unwrap().into_data();
					debug!("SAR received websocket klines update: {:?}", data);
					match serde_json::from_slice::<Value>(&data) {
						Ok(json) =>
							if let Some(open_str) = json.get("o") {
								let open: f64 = open_str.as_str().unwrap().parse().unwrap();
								let high: f64 = json["h"].as_str().unwrap().parse().unwrap();
								let low: f64 = json["l"].as_str().unwrap().parse().unwrap();
								let close: f64 = json["c"].as_str().unwrap().parse().unwrap();
								tx.send(Ohlc { open, high, low, close }).await.unwrap();
							},
						Err(e) => {
							println!("Failed to parse message as JSON: {}", e);
						}
					}
				}
			});

			js.spawn(async move {
				// HACK: shouldn't be unwrapping
				debug!("about to initialise klines");
				let tf_str = tf.try_as_predefined(&BINANCE_TIMEFRAMES).unwrap().to_string();
				let init_klines = crate::exchange_apis::binance::get_historic_klines(symbol.to_string(), tf_str, 100).await.unwrap();
				debug!("initialized klines");
				let init_ohlcs = init_klines.into_iter().map(|k| k.into()).collect::<Vec<Ohlc>>();
				let mut sar = SarIndicator::init(&init_ohlcs, &params_arc.read().unwrap());

				while let Some(ohlc) = rx.recv().await {
					let maybe_order = sar.step(ohlc, &params_arc.read().unwrap(), &symbol, protocol_side);
					if last_order != maybe_order {
						let protocol_spec = params_arc.read().unwrap().to_string();
						tx_orders.send(ProtocolOrders::new(protocol_spec.clone(), vec![maybe_order.clone()])).await.unwrap();
						last_order = maybe_order;
					}
				}
			});
			js.join_all().await;
			Ok(())
		});

		Ok(())
	}

	fn set_params(&self, new_params: Sar) -> Result<()> {
		*self.0.write().unwrap() = new_params;
		Ok(())
	}

	fn get_type(&self) -> ProtocolType {
		ProtocolType::Momentum
	}
}

#[derive(Clone, Copy, Debug, Default, derive_new::new)]
struct SarIndicator {
	sar: f64,
	acceleration_factor: f64,
	extreme_point: f64,
}
impl SarIndicator {
	fn init(init_klines: &[Ohlc], params: &Sar) -> Self {
		let mut sar_indicator = Self {
			sar: init_klines[0].open,
			acceleration_factor: params.start.0,
			extreme_point: init_klines[0].open,
		};
		for ohlc in init_klines {
			_ = sar_indicator.step(*ohlc, params, &Symbol::default(), Side::default());
		}
		sar_indicator
	}

	fn step(&mut self, ohlc: Ohlc, params: &Sar, symbol: &Symbol, side: Side) -> Option<ConceptualOrderPercents> {
		let is_uptrend = self.sar < ohlc.low;

		// Update SAR
		if is_uptrend {
			self.sar = self.sar + self.acceleration_factor * (self.extreme_point - self.sar);
			self.sar = self.sar.min(ohlc.low).min(self.extreme_point);
		} else {
			self.sar = self.sar - self.acceleration_factor * (self.sar - self.extreme_point);
			self.sar = self.sar.max(ohlc.high).max(self.extreme_point);
		}

		// Update extreme point
		if is_uptrend {
			if ohlc.high > self.extreme_point {
				self.extreme_point = ohlc.high;
				self.acceleration_factor = (self.acceleration_factor + *params.increment).min(*params.max);
			}
		} else if ohlc.low < self.extreme_point {
			self.extreme_point = ohlc.low;
			self.acceleration_factor = (self.acceleration_factor + *params.increment).min(*params.max);
		}

		// Check for trend reversal
		if (is_uptrend && ohlc.low < self.sar) || (!is_uptrend && ohlc.high > self.sar) {
			self.sar = self.extreme_point;
			self.extreme_point = if is_uptrend { ohlc.low } else { ohlc.high };
			self.acceleration_factor = *params.start;
		}

		// Compile orders
		let is_followup_side = (side == Side::Sell && is_uptrend) || (side == Side::Buy && !is_uptrend);
		if is_followup_side {
			Some(ConceptualOrderPercents {
				order_type: ConceptualOrderType::StopMarket(ConceptualStopMarket::new(self.sar)),
				symbol: symbol.clone(),
				side,
				qty_percent_of_controlled: Percent::new(1.0),
			})
		} else {
			None
		}
	}
}

#[cfg(test)]
mod tests {
	use v_utils::trades::mock_p_to_ohlc;

	use super::*;

	#[tokio::test]
	async fn test_sar_indicator() {
		let sar_wrapper = SarWrapper::from_str("sar:s0.07:i0.02:m0.15:t1m").unwrap();

		let init_p = v_utils::distributions::laplace_random_walk(100.0, 1000, 0.2, 0.0, Some(123));
		let init_p_reversed = init_p.into_iter().rev().collect::<Vec<_>>();
		let init_ohlc = mock_p_to_ohlc(&init_p_reversed, 10);

		let test_data_p = v_utils::distributions::laplace_random_walk(100.0, 1000, 0.2, 0.0, Some(42));
		let test_data_ohlc = mock_p_to_ohlc(&test_data_p, 10);

		let mut sar_indicator = SarIndicator::init(&init_ohlc, &sar_wrapper.0.read().unwrap());
		let mut recorded_indicator_values = Vec::new();
		let mut orders = Vec::new();

		for (i, ohlc) in test_data_ohlc.into_iter().enumerate() {
			let maybe_order = sar_indicator.step(ohlc, &sar_wrapper.0.read().unwrap(), &Symbol::default(), Side::Sell);
			recorded_indicator_values.push(sar_indicator.sar);
			orders.push((i, maybe_order.map(|o| o.unsafe_stop_market().price)));
		}

		let snapshot = snapshot_fonts::snapshot_plot_orders(&recorded_indicator_values, &orders, false);

		insta::assert_snapshot!(snapshot, @r"
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰫟󻤝󷰕󴟩󱮝󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥107.00
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰨟󴌑󵝙󿿽󿿽󿿽󿿽󻾪󶨌󱪐󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󳘱󹈙󾱡󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿉󼲓󵶑󰷕󰧥󰧥󰧥󰧥󰧥󰧥      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󴿉󼃽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󽪕󰧥󰧥󰨟󴌾󸌱      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰩠󾱍󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󳰙󹐐󾵜󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧱󱘯󸸾󿴌󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󳜭󹊹󲵕󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󵒵󺹉󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󼧙󷰕󳠩󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰩭󹀷󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰪃󿿽󿿽󿿽󻦢󴓎󱹼󰧥󰧥󰧥󰧥󰧥󰧥󰨱󵓟󷸜󸲆󰧥󹦝󿿽󿿽󾭂󽰍󵹍󱽷󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󿿽󿿽󿿽󻚿󵉳󰧥󰧥󰧥󰧥󰧥󷈽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰨹󵱶󳷭󱪐󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰪃󺗉󿿽󿿽󿿽󿿽󿿽󿽹󷶗󰧥󰧥󹢡󺡡󻬕󼁂󿿽󿿽󿿽󽂽󿿽󿿽󿿽󿿽󿿽󿿽󿿜󰧥󰧥󰧥󰧥󰧥󰨨󴰷󺡥󺱺󿿽󿿽󿿽󿿽󿿯󿅉󰧥󰧥󳨡󽖩󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󱒹󴏹󵎹󷨝󹬀󿿽󿿽󿿥󾠹󹾅󶥡󲹑󰧥󰧥󰪷󽣅󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿱󿐖󿰍󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󰧥󰧥󰧥󰧥󵼅󽓙󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿼃󰩆󶦺󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󾤦󻜛󺶡󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󰧥󰧥󴄅󼣝󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿵󿠥󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󰫡󺝥󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽97.46
		──────────────────────────────────────────────────────────────────────────────────────────
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󴏹󸈻󻼺󿌾󿼃󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥105.66
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧻󶝩󺑱󿀽󿿽󿿽󿿽󿿽󿼃󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󲮚󶞝󹫆󼜋      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰨀󲓌󷙹󼄽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿼃󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󾕩󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰫡󶮶󼜟󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿼃󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰨜󴁉󷼌󸈘󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󳐹󹶍󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿼃󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󿿽󿿽󿿽󿿽󿿽      
		󶥡󸣡󹒱󺹉󼌴󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󳰙󹯧󾱡󿿽󿿽󿿽󰧥󰧥󰧥󰧥󰧥󰨹󵳝󺎱󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿼃󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󿿽󿿽󿿽󿿽󿿽      
		󿿽󿿽󿿽󿿽󿼃󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰫡󷔱󽚥󿿽󿿽󿿽󿿽󿿽󿿽󰧥󰧥󰧥󰧥󰧥󰫟󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿼃󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󿿽󿿽󿿽󿿽󿿽97.46
		");
	}
}
