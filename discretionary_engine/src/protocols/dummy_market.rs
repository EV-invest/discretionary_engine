use color_eyre::eyre::Result;
use discretionary_engine_macros::ProtocolWrapper;
use tokio::{sync::mpsc, task::JoinSet};
use v_utils::{Percent, macros as v_macros, trades::Side};

use crate::{
	exchange_apis::{Market, Symbol, order_types::*},
	protocols::{ProtocolOrders, ProtocolTrait, ProtocolType},
};

/// Literally just sends one market order.
#[derive(Clone, v_macros::CompactFormatNamed, Debug, Default, ProtocolWrapper, derive_new::new)]
pub struct DummyMarket {
	// dummy: String, //? Does CompactFormat correctly work with empty structs?
}

impl ProtocolTrait for DummyMarketWrapper {
	type Params = DummyMarket;

	fn attach(&self, set: &mut JoinSet<Result<()>>, tx_orders: mpsc::Sender<ProtocolOrders>, asset: String, protocol_side: Side) -> Result<()> {
		let symbol = Symbol {
			base: asset,
			quote: "USDT".to_owned(),
			market: Market::BinanceFutures,
		};
		let m = ConceptualMarket::new(Percent(1.0));
		let order = ConceptualOrderPercents::new(ConceptualOrderType::Market(m), symbol.clone(), protocol_side, Percent::new(1.0));

		let protocol_spec = self.0.read().unwrap().to_string();
		let protocol_orders = ProtocolOrders::new(protocol_spec, vec![Some(order)]);
		set.spawn(async move {
			tx_orders.send(protocol_orders).await.unwrap();
			//LOOP: it's a dummy protocol, relax
			loop {
				tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
			}
			#[allow(unreachable_code)]
			Ok(())
		});
		Ok(())
	}

	fn set_params(&self, _params: Self::Params) -> Result<()> {
		unimplemented!()
	}

	fn get_type(&self) -> ProtocolType {
		ProtocolType::StopEntry
	}
}
