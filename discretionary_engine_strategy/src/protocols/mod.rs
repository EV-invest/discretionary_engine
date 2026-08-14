//! Protocol definitions and infrastructure.

mod dummy_market;

use std::{collections::HashSet, str::FromStr};

use ahash::AHashSet;
use color_eyre::eyre::{Result, bail};
use dummy_market::DummyMarketWrapper;
use tokio::{sync::mpsc, task::JoinSet};
use tracing::instrument;
use trading_data_core::Side;
use v_utils::Percent;

use crate::order_types::{ConceptualOrder, ConceptualOrderPercents, ProtocolOrderId};

pub trait ProtocolTrait {
	type Params;
	/// Requested orders are being sent over the mspc with uuid of the protocol on each batch, as we want to replace the previous requested batch if any.
	fn attach(&self, set: &mut JoinSet<Result<()>>, tx_orders: mpsc::Sender<ProtocolOrders>, asset: String, protocol_side: Side) -> Result<()>;
	fn set_params(&self, params: Self::Params) -> Result<()>;
	fn get_type(&self) -> ProtocolType;
}
/// Used when determining sizing or the changes in it, in accordance to the current distribution of rm on types of algorithms.
///
/// Size is by default equally distributed amongst the protocols of the same `ProtocolType`, to total 101% for each type with at least one representative.
/// Note that total size is 101% for both the stop and normal orders (because they are on the different sides of the price).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ProtocolType {
	Momentum,
	TP,
	SL,
	StopEntry,
}

#[derive(Clone, Debug)]
pub enum Protocol {
	DummyMarket(DummyMarketWrapper),
}
impl Protocol {
	pub fn attach(&self, position_set: &mut JoinSet<Result<()>>, tx_orders: mpsc::Sender<ProtocolOrders>, asset: String, protocol_side: Side) -> Result<()> {
		match self {
			Protocol::DummyMarket(dm) => dm.attach(position_set, tx_orders, asset, protocol_side),
		}
	}

	pub fn get_type(&self) -> ProtocolType {
		match self {
			Protocol::DummyMarket(dm) => dm.get_type(),
		}
	}

	pub fn signature(&self) -> String {
		match self {
			Protocol::DummyMarket(dm) => dm.signature(),
		}
	}
}

impl FromStr for Protocol {
	type Err = eyre::Report;

	fn from_str(spec: &str) -> Result<Self> {
		if let Ok(dm) = DummyMarketWrapper::from_str(spec) {
			Ok(Protocol::DummyMarket(dm))
		} else {
			bail!("Could not convert string to any Protocol\nString: {spec}")
		}
	}
}

#[instrument]
pub fn interpret_protocol_specs(protocol_specs: Vec<String>) -> Result<Vec<Protocol>> {
	let protocol_specs: Vec<String> = protocol_specs.into_iter().filter(|s| !s.is_empty()).collect();
	if protocol_specs.is_empty() {
		bail!("No protocols specified");
	}
	assert_eq!(protocol_specs.len(), protocol_specs.iter().collect::<HashSet<&String>>().len()); // protocol specs are later used as their IDs
	let mut protocols = Vec::default();
	for spec in protocol_specs {
		let protocol = Protocol::from_str(&spec)?;
		protocols.push(protocol);
	}
	Ok(protocols)
}

/// Wrapper around Orders, which allows for updating the target after a partial fill, without making a new request to the protocol.
///
/// NB: the protocol itself must internally uphold the equality of ids attached to orders to corresponding fields of ProtocolOrders, as well as to ensure that all possible orders the protocol can ever request are initialized in every ProtocolOrders instance it outputs.
#[derive(Clone, Debug, Default)]
pub struct ProtocolOrders {
	pub protocol_id: String,
	pub orders: Vec<Option<ConceptualOrderPercents>>,
}

impl ProtocolOrders {
	#[instrument(skip(orders))]
	pub fn new(protocol_id: String, orders: Vec<Option<ConceptualOrderPercents>>) -> Self {
		assert_ne!(
			orders.len(),
			0,
			"Semantically makes no sense. Protocol must always send Vec<Some<Order>> for all possible orders it will ever send, them being None if at current iteration they are ignored"
		);

		let mut symbols_set: AHashSet<exchange_interactions::Symbol> = AHashSet::default();
		for order in &orders.iter().flatten().collect::<Vec<&ConceptualOrderPercents>>() {
			symbols_set.insert(order.symbol);
		}

		assert_eq!(symbols_set.len(), 1, "Different symbols in return of the same protocol are not yet implemented");
		Self { protocol_id, orders }
	}

	pub fn empty_fills_mask(&self) -> Vec<f64> {
		vec![0.0; self.orders.len()]
	}

	/// Order is *NOT* preserved. Orders with no remaining size are completely excluded from the output.
	#[instrument(skip(self))]
	pub fn recalculate_protocol_orders_allocation(
		&self,
		per_order_infos: &[RecalculateOrdersPerOrderInfo],
		protocol_controlled_notional: f64,
		min_qty_any_ordertype: f64,
	) -> RecalculatedAllocation {
		assert_eq!(self.orders.len(), per_order_infos.len());

		let mut left_controlled_notional = protocol_controlled_notional - per_order_infos.iter().map(|info| info.filled).sum::<f64>();
		// Must be comparing against the largest of min_qties, as we can't force protocols to send their largest order always of the order_type with smallest min_qty.
		if left_controlled_notional < min_qty_any_ordertype {
			return RecalculatedAllocation {
				orders: Vec::default(),
				leftovers: Some(left_controlled_notional),
			};
		}
		let mut per_order_additional_percents_from_skipped = Percent::new(0.0);

		let orders: Vec<ConceptualOrder<ProtocolOrderId>> = self
			.orders
			.iter()
			.enumerate()
			.filter_map(|(i, order)| match order {
				Some(order) => {
					let desired_notional_i = *(order.qty_percent_of_controlled + per_order_additional_percents_from_skipped) * left_controlled_notional;
					if desired_notional_i > per_order_infos[i].min_possible_qty {
						let order = ConceptualOrder::new(ProtocolOrderId::new(self.protocol_id.clone(), i), order.order_type, order.symbol, order.side, desired_notional_i);
						left_controlled_notional -= desired_notional_i;
						Some(order)
					} else {
						per_order_additional_percents_from_skipped += Percent::new(*order.qty_percent_of_controlled / (self.orders.len() - (i + 1)) as f64);
						None
					}
				}
				None => None,
			})
			.collect();

		RecalculatedAllocation { orders, leftovers: None }
	}
}

#[derive(Clone, Debug, Default, derive_new::new)]
pub struct RecalculatedAllocation {
	pub orders: Vec<ConceptualOrder<ProtocolOrderId>>,
	/// None -> protocol is in play
	/// Some -> the remaining value should be redistributed amongst the remaining protocols (of same type if any, otherwise all). Negative value means we overdid it; same rules apply.
	pub leftovers: Option<f64>,
}

#[derive(Clone, Copy, Debug, Default, derive_new::new)]
pub struct RecalculateOrdersPerOrderInfo {
	pub filled: f64,
	pub min_possible_qty: f64,
}

#[cfg(test)]
mod tests {
	use exchange_interactions::core::{Instrument, Symbol};
	use insta::assert_debug_snapshot;
	use trading_data_core::{Pair, Side};
	use v_utils::Percent;

	use super::*;
	use crate::order_types::{ConceptualMarket, ConceptualOrderType};

	mod recalculate_protocol_orders_allocation {
		use super::*;

		fn test_symbol() -> Symbol {
			Symbol::new(Pair::new("BTC".to_string(), "USDT".to_string()), Instrument::Perp)
		}

		#[test]
		fn test_apply_mask() {
			let orders = ProtocolOrders::new(
				"test".to_string(),
				vec![Some(ConceptualOrderPercents::new(
					ConceptualOrderType::Market(ConceptualMarket::new(Percent(1.0))),
					test_symbol(),
					Side::Buy,
					Percent::new(1.0),
				))],
			);

			let protocol_controlled_notional = 2.0;
			let per_order_infos = vec![RecalculateOrdersPerOrderInfo::new(1.1, 0.007)];
			let min_qty_any_ordertype = 0.007;
			let got = orders.recalculate_protocol_orders_allocation(&per_order_infos, protocol_controlled_notional, min_qty_any_ordertype);
			assert_debug_snapshot!(got, @r###"
   RecalculatedAllocation {
       orders: [
           ConceptualOrder {
               id: ProtocolOrderId {
                   protocol_signature: "test",
                   ordinal: 0,
               },
               order_type: Market(
                   ConceptualMarket {
                       maximum_slippage_percent: Percent(
                           1.0,
                       ),
                   },
               ),
               symbol: Symbol {
                   pair: Pair {
                       base: BTC,
                       quote: USDT,
                   },
                   instrument: Perp,
               },
               side: Buy,
               qty_notional: 0.8999999999999999,
           },
       ],
       leftovers: None,
   }
   "###);
		}

		#[test]
		fn nones() {
			let symbol = Symbol::new(Pair::new("ADA".to_string(), "USDT".to_string()), Instrument::Perp);
			let orders = ProtocolOrders::new(
				"test".to_string(),
				vec![
					None,
					Some(ConceptualOrderPercents::new(
						ConceptualOrderType::Market(ConceptualMarket::new(Percent(2.0))),
						symbol,
						Side::Buy,
						Percent::new(1.0),
					)),
				],
			);

			let protocol_controlled_notional = 100.0;
			let per_order_infos = vec![RecalculateOrdersPerOrderInfo::new(0.0, 10.0), RecalculateOrdersPerOrderInfo::new(0.0, 10.0)];
			let min_qty_any_ordertype = 10.0;
			let recalculated_allocation = orders.recalculate_protocol_orders_allocation(&per_order_infos, protocol_controlled_notional, min_qty_any_ordertype);

			let qties = recalculated_allocation.orders.into_iter().map(|co| co.qty_notional).collect::<Vec<f64>>();
			assert_debug_snapshot!(qties, @r###"
  [
      100.0,
  ]
  "###);
		}
	}
}
