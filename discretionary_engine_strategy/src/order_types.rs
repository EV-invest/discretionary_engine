//! Order type definitions for the strategy engine.

use std::hash::Hash;

use color_eyre::eyre::{Result, bail};
use derive_new::new;
use serde::{Deserialize, Serialize};
use v_exchanges::core::Symbol;
use v_utils::{Percent, trades::Side};

pub trait IdRequirements: Hash + Clone + PartialEq + Default + std::fmt::Debug {}
impl<T: Hash + Clone + PartialEq + Default + std::fmt::Debug> IdRequirements for T {}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize, new)]
pub struct Order<Id: IdRequirements> {
	pub id: Id,
	pub order_type: OrderType,
	pub symbol: Symbol,
	pub side: Side,
	pub qty_notional: f64,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub enum OrderType {
	#[default]
	Market,
	StopMarket(StopMarketOrder),
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

/// Generics for defining order types and their whereabouts. Details of execution do not concern us here.
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
	pub fn to_exact<Id: IdRequirements>(self, total_controlled_size: f64, id: Id) -> ConceptualOrder<Id> {
		ConceptualOrder {
			id,
			order_type: self.order_type,
			symbol: self.symbol,
			side: self.side,
			qty_notional: total_controlled_size * *self.qty_percent_of_controlled,
		}
	}
}
