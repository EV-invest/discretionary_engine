use std::hash::Hash;

use color_eyre::eyre::{Result, bail};
use derive_new::new;
use serde::{Deserialize, Serialize};
use trading_data_core::Side;
use v_utils::Percent;

use crate::{PositionOrderId, exchange_apis::Symbol};

// TODO!: Move order_types to v_utils when stable

// TODO!!: automatically derive the Protocol Order types (by substituting `size` with `percent_size`, then auto-implementation of the conversion. Looks like I'm making a `discretionary_engine_macros` crate specifically to for this.

pub trait IdRequirements = Hash + Clone + PartialEq + Default + std::fmt::Debug;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, new)]
pub struct Order<Id: IdRequirements> {
	pub id: Id,
	pub order_type: OrderType,
	pub symbol: Symbol,
	pub side: Side,
	pub qty_notional: f64,
}

/// NB: id of all orders must match uuid field of parent ConceptualOrder if any
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub enum OrderType {
	#[default]
	Market,
	StopMarket(StopMarketOrder),
	// Limit(LimitOrder),
	// StopLimit(StopLimitOrder),
	// TrailingStop(TrailingStopOrder),
	// TWAP(TWAPOrder),
	// Reverse(ReverseOrder),
	// ScaledOrder(ScaledOrder),
	// StopMarket(StopMarketOrder),
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, new)]
pub struct StopMarketOrder {
	pub price: f64,
}

//=============================================================================
// Conceptual Orders
//=============================================================================

#[derive(Clone, Debug, Default, Deserialize, Hash, PartialEq, Serialize, new)]
pub struct ProtocolOrderId {
	pub protocol_signature: String,
	pub ordinal: usize,
}
impl From<PositionOrderId> for ProtocolOrderId {
	fn from(p: PositionOrderId) -> Self {
		ProtocolOrderId {
			protocol_signature: p.protocol_id,
			ordinal: p.ordinal,
		}
	}
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, new)]
pub struct ConceptualOrder<Id: IdRequirements> {
	pub id: Id,
	pub order_type: ConceptualOrderType,
	pub symbol: Symbol,
	pub side: Side,
	pub qty_notional: f64,
}

impl<Id: IdRequirements> ConceptualOrder<Id> {
	pub fn price(&self) -> Result<f64> {
		match &self.order_type {
			ConceptualOrderType::Market(_) => bail!("Market orders don't have a price"),
			ConceptualOrderType::Limit(l) => Ok(l.price),
			ConceptualOrderType::StopMarket(s) => Ok(s.price),
		}
	}
}

/// Generics for defining order types and their whereabouts. Details of execution do not concern us here. We are only trying to specify what we are trying to capture.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
pub enum ConceptualOrderType {
	Market(ConceptualMarket),
	Limit(ConceptualLimit),
	StopMarket(ConceptualStopMarket),
}
impl Default for ConceptualOrderType {
	fn default() -> Self {
		ConceptualOrderType::Market(ConceptualMarket::default())
	}
}

/// Will be executed via above-the-price limits most of the time to prevent excessive slippages.
#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize, new)]
pub struct ConceptualMarket {
	/// 1.0 will be translated into an actual Market order. Others, most of the time, will be expressed via limit orders.
	pub maximum_slippage_percent: Percent,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize, new)]
pub struct ConceptualStopMarket {
	pub price: f64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize, new)]
pub struct ConceptualLimit {
	pub price: f64,
	pub limit_only: bool,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, new)]
pub struct ConceptualOrderPercents {
	pub order_type: ConceptualOrderType,
	pub symbol: Symbol,
	pub side: Side,
	pub qty_percent_of_controlled: Percent,
}
impl ConceptualOrderPercents {
	pub fn to_exact<Id: IdRequirements>(self, total_controled_size: f64, id: Id) -> ConceptualOrder<Id> {
		ConceptualOrder {
			id,
			order_type: self.order_type,
			symbol: self.symbol,
			side: self.side,
			qty_notional: total_controled_size * *self.qty_percent_of_controlled,
		}
	}

	#[doc(hidden)]
	/// # Panics: for use in tests only
	pub fn unsafe_market(&self) -> &ConceptualMarket {
		match &self.order_type {
			ConceptualOrderType::Market(m) => m,
			_ => panic!("Expected Market order, got {:?}", self.order_type),
		}
	}

	#[doc(hidden)]
	/// # Panics: for use in tests only
	pub fn unsafe_limit(&self) -> &ConceptualLimit {
		match &self.order_type {
			ConceptualOrderType::Limit(l) => l,
			_ => panic!("Expected Limit order, got {:?}", self.order_type),
		}
	}

	#[doc(hidden)]
	/// # Panics: for use in tests only
	pub fn unsafe_stop_market(&self) -> &ConceptualStopMarket {
		match &self.order_type {
			ConceptualOrderType::StopMarket(s) => s,
			_ => panic!("Expected StopMarket order, got {:?}", self.order_type),
		}
	}
}
