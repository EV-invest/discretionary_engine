use std::collections::HashMap;

use miette::Result;
use uuid::Uuid;
use v_exchanges::{ExchangeName, Symbol, orders::LimitOrder};
use v_utils::trades::Side;

#[derive(clap::Args, Debug)]
pub struct ConceptualLimitArgs {
	/// follows rules for normal limit orders
	#[arg(long)]
	pub limit: f32,

	/// gimme [Symbol](v_exchanges::Symbol)
	#[arg(long)]
	pub symbol: Symbol,

	/// qty size
	#[arg(long)]
	pub size: f32,

	/// side
	#[arg(long)]
	pub side: Side,
	//TODO: the actually juicy parts like the relative cost of price diff vs time
}

#[derive(Clone, Debug)]
pub struct ConceptualLimit {
	pub id: Uuid,

	limit: f32,
	symbol: Symbol,
	size_q: f32,
	side: Side,

	/// total per-exchange qty fill value
	filled: HashMap<ExchangeName, f32>,
}
impl ConceptualLimit {
	/// produces the vec of exact target orders that we want to see currently outstanding
	///
	/// no generics or "semantic" stuff at this level, - we produce exact limit orders for exact exchange with exact configuration  
	pub async fn next(&self) -> Result<Vec<LimitOrder>, Error> {
		todo!()
	}
}

#[derive(Debug, derive_more::Display, thiserror::Error, derive_more::From)]
/// Error during the conversion of intent into exact orders
pub enum Error {
	Other(miette::Report),
}

impl From<ConceptualLimitArgs> for ConceptualLimit {
	fn from(v: ConceptualLimitArgs) -> Self {
		ConceptualLimit {
			limit: v.limit,
			symbol: v.symbol,
			size_q: v.size,
			id: Uuid::now_v7(),
			side: v.side,
		}
	}
}
