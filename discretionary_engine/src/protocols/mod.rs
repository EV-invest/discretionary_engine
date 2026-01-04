mod approaching_limit;
mod dummy_market;
mod sar;
mod trailing_stop;
use std::{collections::HashSet, str::FromStr};

use approaching_limit::{ApproachingLimit, ApproachingLimitWrapper};
use color_eyre::eyre::{Result, bail};
use dummy_market::DummyMarketWrapper;
use sar::{Sar, SarWrapper};
use tokio::{sync::mpsc, task::JoinSet};
use tracing::instrument;
use trailing_stop::{TrailingStop, TrailingStopWrapper};
use uuid::Uuid;
use v_utils::{Percent, trades::Side};

use crate::exchange_apis::order_types::{ConceptualOrder, ConceptualOrderPercents, ProtocolOrderId};

/// Used when determining sizing or the changes in it, in accordance to the current distribution of rm on types of algorithms.
///
/// Size is by default equally distributed amongst the protocols of the same `ProtocolType`, to total 101% for each type with at least one representative.
/// Note that total size is is 101% for both the stop and normal orders (because they are on the different sides of the price).
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, derive_new::new)]
pub enum ProtocolType {
	Momentum,
	TP,
	SL,
	StopEntry,
}

pub trait ProtocolTrait {
	type Params;
	/// Requested orders are being sent over the mspc with uuid of the protocol on each batch, as we want to replace the previous requested batch if any.
	fn attach(&self, set: &mut JoinSet<Result<()>>, tx_orders: mpsc::Sender<ProtocolOrders>, asset: String, protocol_side: Side) -> Result<()>;
	fn set_params(&self, params: Self::Params) -> Result<()>;
	fn get_type(&self) -> ProtocolType;
}

// HACK: Protocol enum. Seems suboptimal {\{{
//TODO!!!!!: pretty sure we can just make a dyn trait. Might need to use `dyno` crate or sth.
#[derive(Clone, Debug)]
pub enum Protocol {
	TrailingStop(TrailingStopWrapper),
	Sar(SarWrapper),
	ApproachingLimit(ApproachingLimitWrapper),
	DummyMarket(DummyMarketWrapper),
}
impl Protocol {
	pub fn attach(&self, position_set: &mut JoinSet<Result<()>>, tx_orders: mpsc::Sender<ProtocolOrders>, asset: String, protocol_side: Side) -> Result<()> {
		match self {
			Protocol::TrailingStop(ts) => ts.attach(position_set, tx_orders, asset, protocol_side),
			Protocol::Sar(sar) => sar.attach(position_set, tx_orders, asset, protocol_side),
			Protocol::ApproachingLimit(al) => al.attach(position_set, tx_orders, asset, protocol_side),
			Protocol::DummyMarket(dm) => dm.attach(position_set, tx_orders, asset, protocol_side),
		}
	}

	pub fn set_params(&self, params: ProtocolParams) -> Result<()> {
		match self {
			Protocol::TrailingStop(ts) => match params {
				ProtocolParams::TrailingStop(ts_params) => ts.set_params(ts_params),
				_ => Err(eyre::Report::msg("Mismatched params")),
			},
			Protocol::Sar(sar) => match params {
				ProtocolParams::Sar(sar_params) => sar.set_params(sar_params),
				_ => Err(eyre::Report::msg("Mismatched params")),
			},
			Protocol::ApproachingLimit(al) => match params {
				ProtocolParams::ApproachingLimit(al_params) => al.set_params(al_params),
				_ => Err(eyre::Report::msg("Mismatched params")),
			},
			Protocol::DummyMarket(_) => Ok(()),
		}
	}

	pub fn get_type(&self) -> ProtocolType {
		match self {
			Protocol::TrailingStop(ts) => ts.get_type(),
			Protocol::Sar(sar) => sar.get_type(),
			Protocol::ApproachingLimit(al) => al.get_type(),
			Protocol::DummyMarket(dm) => dm.get_type(),
		}
	}

	pub fn signature(&self) -> String {
		match self {
			Protocol::TrailingStop(ts) => ts.signature(),
			Protocol::Sar(sar) => sar.signature(),
			Protocol::ApproachingLimit(al) => al.signature(),
			Protocol::DummyMarket(dm) => dm.signature(),
		}
	}
}
impl FromStr for Protocol {
	type Err = eyre::Report;

	fn from_str(spec: &str) -> Result<Self> {
		if let Ok(ts) = TrailingStopWrapper::from_str(spec) {
			Ok(Protocol::TrailingStop(ts))
		} else if let Ok(sar) = SarWrapper::from_str(spec) {
			Ok(Protocol::Sar(sar))
		} else if let Ok(al) = ApproachingLimitWrapper::from_str(spec) {
			Ok(Protocol::ApproachingLimit(al))
		} else if let Ok(dm) = DummyMarketWrapper::from_str(spec) {
			Ok(Protocol::DummyMarket(dm))
		} else {
			bail!("Could not convert string to any Protocol\nString: {spec}")
		}
	}
}

#[derive(Clone, Debug, derive_new::new)]
pub enum ProtocolParams {
	TrailingStop(TrailingStop),
	Sar(Sar),
	ApproachingLimit(ApproachingLimit),
}
impl From<TrailingStop> for ProtocolParams {
	fn from(ts: TrailingStop) -> Self {
		ProtocolParams::TrailingStop(ts)
	}
}
impl From<Sar> for ProtocolParams {
	fn from(sar: Sar) -> Self {
		ProtocolParams::Sar(sar)
	}
}
impl From<ApproachingLimit> for ProtocolParams {
	fn from(al: ApproachingLimit) -> Self {
		ProtocolParams::ApproachingLimit(al)
	}
}
//,}}}

#[instrument]
pub fn interpret_protocol_specs(protocol_specs: Vec<String>) -> Result<Vec<Protocol>> {
	let protocol_specs: Vec<String> = protocol_specs.into_iter().filter(|s| s != "").collect();
	if protocol_specs.len() == 0 {
		bail!("No protocols specified");
	}
	assert_eq!(protocol_specs.len(), protocol_specs.iter().collect::<HashSet<&String>>().len()); // protocol specs are later used as their IDs
	let mut protocols = Vec::new();
	for spec in protocol_specs {
		let protocol = Protocol::from_str(&spec)?;
		protocols.push(protocol);
	}
	Ok(protocols)
}

#[derive(Clone, Debug, Default, derive_new::new)]
pub struct ProtocolFill {
	pub id: ProtocolOrderId,
	pub qty: f64,
}

#[derive(Clone, Debug, Default, derive_new::new)]
pub struct ProtocolFills {
	pub key: Uuid,
	pub fills: Vec<ProtocolFill>,
}

/// Position's knowledge of the protocols in use.
#[derive(Clone, Debug, Default)]
pub struct ProtocolDynamicInfo {
	pub fills: Vec<f64>,
	pub protocol_orders: ProtocolOrders,
}
impl ProtocolDynamicInfo {
	pub fn new(protocol_orders: ProtocolOrders) -> Self {
		let fills = protocol_orders.empty_fills_mask();
		Self { fills, protocol_orders }
	}

	pub fn update_fills(&mut self, fills: Vec<f64>) {
		self.fills = fills;
	}

	pub fn update_fill_at(&mut self, i: usize, fill: f64) {
		self.fills[i] += fill;
	}

	pub fn update_orders(&mut self, orders: ProtocolOrders) {
		self.protocol_orders = orders;
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

/// Wrapper around Orders, which allows for updating the target after a partial fill, without making a new request to the protocol.
///
/// # Contract
/// - [impl protocol.orders.id-stability]: Protocol must maintain stable order IDs - ordinal indices must remain constant
/// - [impl protocol.orders.id-match]: ProtocolOrderId must match protocol signature and ordinal position
/// - [impl protocol.orders.all-slots-initialized]: All possible orders must have slots, inactive ones as None
#[derive(Clone, Debug, Default)]
pub struct ProtocolOrders {
	pub protocol_id: String,
	pub __orders: Vec<Option<ConceptualOrderPercents>>, // pub for testing purposes
}
impl ProtocolOrders {
	#[instrument(skip(orders))]
	pub fn new(protocol_id: String, orders: Vec<Option<ConceptualOrderPercents>>) -> Self {
		// [verify protocol.orders.all-slots-initialized]
		assert_ne!(
			orders.len(),
			0,
			"Semantically makes no sense. Protocol must always send Vec<Some<Order>> for all possible orders it will ever send, them being None if at current iteration they are ignored"
		);

		let mut symbols_set = HashSet::new();
		for order in &orders.iter().flatten().collect::<Vec<&ConceptualOrderPercents>>() {
			symbols_set.insert(order.symbol.clone());
		}

		assert_eq!(symbols_set.len(), 1, "Different symbols in return of the same protocol are not yet implemented");
		Self { protocol_id, __orders: orders }
	}

	pub fn empty_fills_mask(&self) -> Vec<f64> {
		vec![0.0; self.__orders.len()]
	}

	/// Order is *NOT* preserved. Orders with no remaining size are completely excluded from the output.
	///
	//HACK: doesn't yet work with multiple symbols.
	// Matter of fact, none of this does. Currently all Positions assume working with specific asset.
	#[instrument(skip(self))]
	pub fn recalculate_protocol_orders_allocation(
		&self,
		per_order_infos: &[RecalculateOrdersPerOrderInfo],
		protocol_controlled_notional: f64,
		min_qty_any_ordertype: f64,
	) -> RecalculatedAllocation {
		assert_eq!(self.__orders.len(), per_order_infos.len());

		let mut left_controlled_notional = protocol_controlled_notional - per_order_infos.iter().map(|info| info.filled).sum::<f64>();
		// Must be comparing against the largest of min_qties, as we can't force protocols to send their largest order always of the order_type with smallest min_qty.
		if left_controlled_notional < min_qty_any_ordertype {
			return RecalculatedAllocation {
				orders: Vec::new(),
				leftovers: Some(left_controlled_notional),
			};
		}
		let mut per_order_additional_percents_from_skipped = Percent::new(0.0);

		let orders: Vec<ConceptualOrder<ProtocolOrderId>> = self
			.__orders
			.iter()
			.enumerate()
			.filter_map(|(i, order)| match order {
				Some(order) => {
					let desired_notional_i = *(order.qty_percent_of_controlled + per_order_additional_percents_from_skipped) * left_controlled_notional;
					if desired_notional_i > per_order_infos[i].min_possible_qty {
						let order = ConceptualOrder::new(
							ProtocolOrderId::new(self.protocol_id.clone(), i),
							order.order_type,
							order.symbol.clone(),
							order.side,
							desired_notional_i,
						);
						left_controlled_notional -= desired_notional_i;
						Some(order)
					} else {
						per_order_additional_percents_from_skipped += Percent::new(*order.qty_percent_of_controlled / (self.__orders.len() - (i + 1)) as f64);
						None
					}
				}
				None => None,
			})
			.collect();

		RecalculatedAllocation { orders, leftovers: None }
	}
}

#[cfg(test)]
mod tests {
	use insta::assert_debug_snapshot;
	use lazy_static::lazy_static;
	use v_utils::{Percent, trades::Side};

	use super::*;
	use crate::exchange_apis::{
		Market, Symbol,
		order_types::{ConceptualMarket, ConceptualOrderType},
	};

	mod recalculate_protocol_orders_allocation {
		use super::*;

		#[test]
		fn test_apply_mask() {
			let orders = ProtocolOrders::new(
				"test".to_string(),
				vec![Some(ConceptualOrderPercents::new(
					ConceptualOrderType::Market(ConceptualMarket::new(Percent(1.0))),
					Symbol::new("BTC".to_string(), "USDT".to_string(), Market::BinanceFutures),
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
                   base: "BTC",
                   quote: "USDT",
                   market: BinanceFutures,
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
			let orders = ProtocolOrders::new(
				"test".to_string(),
				vec![
					None,
					Some(ConceptualOrderPercents::new(
						ConceptualOrderType::Market(ConceptualMarket::new(Percent(2.0))),
						Symbol::new("ADA".to_string(), "USDT".to_string(), Market::BinanceFutures),
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

		mod two_diff_orders {
			use super::*;

			lazy_static! {
				static ref ORDERS: ProtocolOrders = ProtocolOrders::new(
					"test".to_string(),
					vec![
						Some(ConceptualOrderPercents::new(
							ConceptualOrderType::Market(ConceptualMarket::new(Percent(1.0))),
							Symbol::new("ADA".to_string(), "USDT".to_string(), Market::BinanceFutures),
							Side::Sell,
							Percent::new(0.25),
						)),
						Some(ConceptualOrderPercents::new(
							ConceptualOrderType::Market(ConceptualMarket::new(Percent(1.0))),
							Symbol::new("ADA".to_string(), "USDT".to_string(), Market::BinanceFutures),
							Side::Buy,
							Percent::new(0.75),
						)),
					],
				);
				static ref MIN_QTY_ANY_ORDERTYPE: f64 = 10.0;
			}

			#[test]
			fn full_fill() {
				let protocol_controlled_notional = 100.0;
				let per_order_infos = vec![RecalculateOrdersPerOrderInfo::new(75.0, 10.0), RecalculateOrdersPerOrderInfo::new(25.0, 10.0)];
				let got = ORDERS.recalculate_protocol_orders_allocation(&per_order_infos, protocol_controlled_notional, *MIN_QTY_ANY_ORDERTYPE);
				assert_debug_snapshot!(got, @r###"
    RecalculatedAllocation {
        orders: [],
        leftovers: Some(
            0.0,
        ),
    }
    "###);
			}

			#[test]
			fn overfill() {
				let protocol_controlled_notional = 2.0;
				let per_order_infos = vec![RecalculateOrdersPerOrderInfo::new(25.0, 10.0), RecalculateOrdersPerOrderInfo::new(25.0, 10.0)];
				let got = ORDERS.recalculate_protocol_orders_allocation(&per_order_infos, protocol_controlled_notional, *MIN_QTY_ANY_ORDERTYPE);
				//TODO!!!: start returning by how much we were off. In the next snapshot we overdo it by 48.0, yet all we get is a success with empty vec of orders to deploy.
				assert_debug_snapshot!(got, @r###"
    RecalculatedAllocation {
        orders: [],
        leftovers: Some(
            -48.0,
        ),
    }
    "###);
			}

			#[test]
			fn size_redistribution_on_hitting_min_qty() {
				let protocol_controlled_notional = 15.0;
				let per_order_infos = vec![RecalculateOrdersPerOrderInfo::new(0.0, 10.0), RecalculateOrdersPerOrderInfo::new(0.0, 10.0)];
				let got = ORDERS.recalculate_protocol_orders_allocation(&per_order_infos, protocol_controlled_notional, *MIN_QTY_ANY_ORDERTYPE);
				//TODO!!!: start returning by how much we were off. In the next snapshot we overdo it by 48.0, yet all we get is a success with empty vec of orders to deploy.
				assert_debug_snapshot!(got, @r###"
    RecalculatedAllocation {
        orders: [
            ConceptualOrder {
                id: ProtocolOrderId {
                    protocol_signature: "test",
                    ordinal: 1,
                },
                order_type: Market(
                    ConceptualMarket {
                        maximum_slippage_percent: Percent(
                            1.0,
                        ),
                    },
                ),
                symbol: Symbol {
                    base: "ADA",
                    quote: "USDT",
                    market: BinanceFutures,
                },
                side: Buy,
                qty_notional: 15.0,
            },
        ],
        leftovers: None,
    }
    "###);
			}
		}
	}
}
