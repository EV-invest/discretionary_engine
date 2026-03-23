use std::{
	collections::HashMap,
	hash::{Hash, Hasher},
};

use miette::Result;
use uuid::Uuid;
use v_exchanges::{ExchangeName, ExchangeOrder, Symbol, orders::LimitOrder};
use v_utils::trades::Side;

use crate::data::BookHandle;

#[derive(clap::Args, Debug, serde::Deserialize, serde::Serialize)]
pub struct ConceptualLimitChangeable {
	/// follows rules for normal limit orders
	#[arg(long)]
	pub limit: f64,

	/// qty size (signed, - side inferred)
	#[arg(long)]
	pub qty: f64,
	//TODO: the actually juicy parts like the relative cost of price diff vs time
}

#[derive(clap::Args, Debug, serde::Deserialize, serde::Serialize)]
pub struct ConceptualLimitArgs {
	/// gimme [Symbol](v_exchanges::Symbol)
	#[arg(long)]
	pub symbol: Symbol,

	#[command(flatten)]
	pub changeable: ConceptualLimitChangeable,
}

#[derive(Debug, derive_new::new)]
pub struct ConceptualLimit {
	pub id: Uuid,

	limit: f64,
	symbol: Symbol,
	size_q: f64,
	side: Side,

	/// total per-exchange qty fill value
	__filled: HashMap<ExchangeName, f64>,
	__book: BookHandle,
}
impl ConceptualLimit {
	pub fn adjust(&mut self, adj: ConceptualLimitChangeable) -> std::result::Result<(), crate::InvalidRoutingError> {
		let (size, side) = match adj.qty {
			p if p > 0. => (p, Side::Buy),
			p if p < 0. => (-p, Side::Sell),
			_ => unreachable!("should've checked before here, - where we still can report to user"),
		};

		let total_filled: f64 = self.__filled.values().map(|v| *v as f64).sum();
		if side != self.side && total_filled > 0.0 {
			//return Err(crate::InvalidRoutingError::AdjustmentWouldReverse); //Q: do we care to go freak out and go into HobbleMode when we simply need to revrese the position?
			tracing::warn!("requested adjustment reverses the acquisition direction. Might not be intentional or desirable.\nAlready have {total_filled:?}")
		}

		self.limit = adj.limit;
		self.size_q = size;
		self.side = side;
		Ok(())
	}

	/// produces the vec of exact target orders that we want to see currently outstanding
	///
	/// no generics or "semantic" stuff at this level, - we produce exact limit orders for exact exchange with exact configuration
	pub async fn next(&self) -> Result<Vec<ExchangeOrder<LimitOrder>>, Error> {
		let book = self.__book.snapshot();

		//HACK: the dumbest Chase Limit imaginable
		//TODO!!!: make proper
		let price = match self.side {
			Side::Buy => {
				let best_bid = book.bids.iter().map(|(p, _)| *p).fold(f64::NEG_INFINITY, f64::max);
				if best_bid == f64::NEG_INFINITY { self.limit } else { best_bid.min(self.limit) }
			}
			Side::Sell => {
				let best_ask = book.asks.iter().map(|(p, _)| *p).fold(f64::INFINITY, f64::min);
				if best_ask == f64::INFINITY { self.limit } else { best_ask.max(self.limit) }
			}
		};

		let limit = LimitOrder::new(self.side, price, self.size_q);
		let exchg = ExchangeOrder {
			order: limit,
			ticker: v_exchanges::Ticker {
				symbol: self.symbol,
				exchange_name: ExchangeName::Bybit, //dbg
			},
			expected_fee_usd: None,
		};
		Ok(vec![exchg])
	}

	pub(crate) fn from_args(v: ConceptualLimitArgs, book: BookHandle) -> Self {
		let (size, side) = match v.changeable.qty {
			p if p > 0. => (p, Side::Buy),
			p if p < 0. => (-p, Side::Sell),
			_ => unreachable!("should've checked before here, - where we still can report to user"),
		};
		ConceptualLimit {
			side,
			limit: v.changeable.limit,
			size_q: size,
			symbol: v.symbol,

			id: Uuid::now_v7(),
			__filled: HashMap::new(),
			__book: book,
		}
	}
}

impl PartialEq for ConceptualLimit {
	fn eq(&self, other: &Self) -> bool {
		self.id == other.id
	}
}
impl Eq for ConceptualLimit {}
impl Hash for ConceptualLimit {
	fn hash<H: Hasher>(&self, state: &mut H) {
		self.id.hash(state);
	}
}

//TODO: move to v_exchanges
// want to have an object fully encompasing all the relevant physical updates
#[derive(Debug, derive_more::Display, thiserror::Error, derive_more::From)]
/// Error during the conversion of intent into exact orders
pub enum Error {
	Other(miette::Report),
}
