//! DummyMarket protocol - sends a single market order.

use std::{fmt, str::FromStr};

use color_eyre::eyre::Result;
use de_macros::ProtocolWrapper;
use tokio::{sync::mpsc, task::JoinSet};
use v_exchanges::core::{Instrument, Symbol};
use v_utils::{
	Percent,
	trades::{Pair, Side},
};

use super::{ProtocolOrders, ProtocolTrait, ProtocolType};
use crate::order_types::{ConceptualMarket, ConceptualOrderPercents, ConceptualOrderType};

/// Literally just sends one market order.
#[derive(Clone, Debug, Default, ProtocolWrapper, derive_new::new)]
pub struct DummyMarket {}

impl fmt::Display for DummyMarket {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "dm")
	}
}

impl FromStr for DummyMarket {
	type Err = eyre::Report;

	fn from_str(spec: &str) -> Result<Self> {
		let prefix = spec.split(':').next().unwrap_or(spec);
		if prefix != "dm" {
			eyre::bail!("expected prefix 'dm', got '{prefix}'");
		}
		let rest = spec.strip_prefix("dm").unwrap_or("");
		let rest = rest.strip_prefix(':').unwrap_or(rest);
		if !rest.is_empty() {
			eyre::bail!("DummyMarket takes no parameters, got '{rest}'");
		}
		Ok(Self {})
	}
}

impl ProtocolTrait for DummyMarketWrapper {
	type Params = DummyMarket;

	fn attach(&self, set: &mut JoinSet<Result<()>>, tx_orders: mpsc::Sender<ProtocolOrders>, asset: String, protocol_side: Side) -> Result<()> {
		let symbol = Symbol::new(Pair::new(asset, "USDT".to_string()), Instrument::Perp);
		let m = ConceptualMarket::new(Percent(1.0));
		let order = ConceptualOrderPercents::new(ConceptualOrderType::Market(m), symbol, protocol_side, Percent::new(1.0));

		let protocol_spec = self.0.read().unwrap().to_string();
		let protocol_orders = ProtocolOrders::new(protocol_spec, vec![Some(order)]);
		set.spawn(async move {
			tx_orders.send(protocol_orders).await.unwrap();
			// LOOP: it's a dummy protocol, relax
			loop {
				tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
			}
			#[expect(unreachable_code)]
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

#[cfg(test)]
mod tests {
	use std::str::FromStr;

	use super::*;

	#[test]
	fn parse_dm() {
		let dm = DummyMarketWrapper::from_str("dm").unwrap();
		assert_eq!(dm.signature(), "dm");
	}

	#[test]
	fn parse_dm_with_colon() {
		let dm = DummyMarketWrapper::from_str("dm:").unwrap();
		assert_eq!(dm.signature(), "dm");
	}

	#[test]
	fn parse_dm_with_params_fails() {
		let result = DummyMarketWrapper::from_str("dm:p0.5");
		assert!(result.is_err());
	}

	#[test]
	fn parse_wrong_prefix_fails() {
		let result = DummyMarketWrapper::from_str("ts:p0.5");
		assert!(result.is_err());
	}
}
