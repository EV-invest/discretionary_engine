use color_eyre::eyre::Result;
use de_macros::ProtocolWrapper;
use futures_util::StreamExt;
use serde_json::Value;
use tokio::{sync::mpsc, task::JoinSet};
use tokio_tungstenite::connect_async;
use trading_data_core::Side;
use v_utils::{Percent, macros::CompactFormat};

use crate::{
	exchange_apis::{Market, Symbol, order_types::*},
	protocols::{ProtocolOrders, ProtocolTrait, ProtocolType},
};

#[derive(Clone, CompactFormat, Copy, Debug, Default, ProtocolWrapper, derive_new::new)]
pub struct TrailingStop {
	percent: Percent,
}

impl ProtocolTrait for TrailingStopWrapper {
	type Params = TrailingStop;

	fn attach(&self, position_js: &mut JoinSet<Result<()>>, tx_orders: mpsc::Sender<ProtocolOrders>, asset: String, protocol_side: Side) -> Result<()> {
		let symbol = Symbol {
			base: asset,
			quote: "USDT".to_owned(),
			market: Market::BinanceFutures,
		};
		let address = format!("wss://fstream.binance.com/ws/{}@aggTrade", symbol.to_string().to_lowercase());

		let (tx, mut rx) = tokio::sync::mpsc::channel::<f64>(256);
		let params = self.0.clone();
		position_js.spawn(async move {
			let mut js = JoinSet::new();
			js.spawn(async move {
				let (ws_stream, _) = connect_async(address).await.unwrap();
				let (_, mut read) = ws_stream.split();

				while let Some(msg) = read.next().await {
					let data = msg.unwrap().into_data();
					match serde_json::from_slice::<Value>(&data) {
						Ok(json) =>
							if let Some(price_str) = json.get("p") {
								let price: f64 = price_str.as_str().unwrap().parse().unwrap();
								tx.send(price).await.unwrap();
							},
						Err(e) => {
							println!("Failed to parse message as JSON: {e}");
						}
					}
				}
			});

			js.spawn(async move {
				let mut ts_indicator = TrailingStopIndicator::new();
				while let Some(price) = rx.recv().await {
					let maybe_order = ts_indicator.step(price, params.read().unwrap().percent, protocol_side, &symbol);
					if let Some(order) = maybe_order {
						let protocol_spec = params.read().unwrap().to_string();
						let protocol_orders = ProtocolOrders::new(protocol_spec, vec![Some(order)]);
						tx_orders.send(protocol_orders).await.unwrap();
					}
				}
			});
			js.join_all().await;
			Ok(())
		});
		Ok(())
	}

	fn set_params(&self, new_params: TrailingStop) -> Result<()> {
		*self.0.write().unwrap() = new_params;
		Ok(())
	}

	fn get_type(&self) -> ProtocolType {
		ProtocolType::Momentum
	}
}

#[derive(Clone, Copy, Debug, Default)]
struct TrailingStopIndicator {
	top: f64,
	bottom: f64,
}
impl TrailingStopIndicator {
	fn new() -> Self {
		Self { top: 0.0, bottom: 0.0 }
	}

	fn step(&mut self, price: f64, percent: Percent, side: Side, symbol: &Symbol) -> Option<ConceptualOrderPercents> {
		if price < self.bottom || self.bottom == 0.0 {
			self.bottom = price;
			if side == Side::Buy {
				let target_price = price * ((1.0_f64 + percent.abs()).ln() + 1.0);
				let sm = ConceptualStopMarket::new(target_price);
				let order = Some(ConceptualOrderPercents::new(ConceptualOrderType::StopMarket(sm), symbol.clone(), Side::Buy, Percent::new(1.0)));
				return order;
			}
		}
		if price > self.top || self.top == 0.0 {
			self.top = price;
			if side == Side::Sell {
				let target_price = price * ((1.0_f64 - percent.abs()).ln() + 1.0);
				let sm = ConceptualStopMarket::new(target_price);
				let order = Some(ConceptualOrderPercents::new(ConceptualOrderType::StopMarket(sm), symbol.clone(), Side::Sell, Percent::new(1.0)));
				return order;
			}
		}
		None
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[tokio::test]
	async fn internals() {
		let mut ts = TrailingStopIndicator::default();
		let mut orders = Vec::default();
		let prices = v_utils::distributions::laplace_random_walk(100.0, 1000, 0.1, 0.0, Some(42));
		for (i, price) in prices.iter().enumerate() {
			if let Some(order) = ts.step(*price, Percent(0.02), Side::Sell, &Symbol::new("BTC", "USDT", Market::BinanceFutures)) {
				let ConceptualOrderPercents { order_type, .. } = order;
				if let ConceptualOrderType::StopMarket(sm) = order_type {
					orders.push((i, Some(sm.price)));
				} else {
					panic!("Expected StopMarket order type");
				}
			}
		}
		let plot = snapshot_fonts::snapshot_plot_orders(&prices, &orders, false);
		insta::assert_snapshot!(plot, @r"
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󳎶󶅇󶭞󴞪󰧥󰧲󰧥󰨫󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥103.50
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󵾊󰧥󰩌󿿽󿿽󿿽󿿘󿌯󻭛󱨃󻊈󼔁󸎂󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󹁽󿿽󺂇󺺜󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󵸷󰧥󰧥󰧥󰧥󰧥󰧥󰧥󵞂󻇶󷆰󸒛      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧴󷦎󶩡󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿾴󺄞󹽴󻃋󴀁󰧥󰧥󲸠󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰪮󹶼󹨅󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󸪉󼟬󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧵󾹗󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰩰󲴱󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰨅󴜪󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰨏󸱧󷰁󲆇󸍹󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󱮊󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󺖢󾩫󿿺󵼲󲉳󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧿󲦑󱽷󰧥󰧥󹨅󿿽󼷋󸌤󰧥󰧥󰨦󱈦󰧻󾖄󿿽󿿽󿿽󿾓󲎄󰧭󺊼󰧥󰧥󰧥󰧥󵇖󹺟󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󳫫󵱁󰧥󰧩󴺇󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󷲒󿿽󿿽󿿽󿾷󿿽󽧁󰧥󰧥󰧥󰩶󰨪󻛄󻯏󼠚󿿽󿿹󼣂󻩌󿿽󿿽󿿽󿿽󻘩󺎈󺲦󿿽󾂝󿿽󿿽󿿽󿿽󿿽󿿽󽪻󿿽󼼓󰧥󳠠󰹸󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󿾳󿾽󵐆󹌨󿿥󼕪󻥻󰧥󵭆󰧥󰧥󰧥󰧥󰧥󰧥󰩱󿿽󿿽󿿽󿿽󿿽󿿽󿼳󽚂󰧥󽵂󿰑󹳶󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󻜋󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿘟󿿄󼝚󵾛󰧥󱆽󰧥󰩬󼜟󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󸈙󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󽋃󿿤󼉝󽧀󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽98.73
		──────────────────────────────────────────────────────────────────────────────────────────
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󵷱󻰑󻰑󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽101.41
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󱖵󻌵󻌵󻍁󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰻳󵫏󸯕󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰪋󺵎󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧳󱞭󱞭󱞭󱞭󱞭󱞭󱞭󱞭󱞭󱞭󱞭󱟥󿠥󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󰧥󲶣󹣫󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾅹󾆏󿜡󿜡󿜡󿜡󿜡󿜡󿜡󿜡󿜡󿜪󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽      
		󰫶󲕵󲕵󲕵󲕵󲕵󲕵󲕵󲕵󲕵󲕵󲕵󲕵󲕵󲕵󲕵󳽯󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽󿿽97.98
		");
	}
}
