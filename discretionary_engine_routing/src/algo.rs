use std::hash::{Hash, Hasher};

use ahash::AHashMap;
use miette::Result;
use uuid::Uuid;
use v_exchanges::{ExchangeName, ExchangeOrder, Symbol, orders::LimitOrder};
use v_utils::{
	arch::{Keyed, MyKey},
	trades::Side,
};

use crate::data::BookRef;

#[derive(Clone, Debug, clap::Args, serde::Deserialize, serde::Serialize)]
pub struct ConceptualLimitChangeable {
	/// follows rules for normal limit orders
	#[arg(long)]
	pub limit: f64,

	/// qty size (signed, - side inferred)
	#[arg(long)]
	pub qty: f64,

	//Q: Tried to impl a termination special-cases (like saying we don't care to continue when price comes back after leaving the range). But then it has to depend on how far away or how deep inside it has had gotten. But then this starts to sound pretty generic, so might as well just implement it for everyone?
	//#[arg(long, value_enum)]
	//pub termination: ConceptualLimitTermination,
	//Q: what about a scaling gradient for how violently we react

	//pub aggression: ?, //Q: want some way to express how aggressive the Iceberg part should be
	#[arg(short, long)]
	pub time: jiff::SignedDuration,
}

//#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, clap::ValueEnum)]
//pub enum ConceptualLimitTermination {
//	/// Allotted time has run out
//	Timeout,
//	/// First cross of the limit price since ently
//	Crossed,
//}

#[derive(Clone, Debug, clap::Args, serde::Deserialize, serde::Serialize)]
pub struct ConceptualLimitArgs {
	/// gimme [Symbol](v_exchanges::Symbol)
	#[arg(short, long)]
	pub symbol: Symbol,

	#[command(flatten)]
	pub changeable: ConceptualLimitChangeable,
}

#[derive(Clone, Debug)]
pub struct ConceptualLimit {
	pub id: Uuid,

	limit: f64,
	symbol: Symbol,
	size_q: f64,
	side: Side,

	/// total per-exchange qty fill value
	__book: BookRef,
	__filled: AHashMap<ExchangeName, f64>,
	__prev: Vec<ExchangeOrder<LimitOrder>>,
}
impl ConceptualLimit {
	pub fn adjust(&mut self, adj: ConceptualLimitChangeable) -> std::result::Result<(), crate::InvalidRoutingError> {
		let (size, side) = match adj.qty {
			p if p > 0. => (p, Side::Buy),
			p if p < 0. => (-p, Side::Sell),
			_ => unreachable!("should've checked before here, - where we still can report to user"),
		};

		let total_filled: f64 = self.__filled.values().copied().sum();
		if side != self.side && total_filled > 0.0 {
			//return Err(crate::InvalidRoutingError::AdjustmentWouldReverse); //Q: do we care to go freak out and go into HobbleMode when we simply need to revrese the position?
			tracing::warn!("requested adjustment reverses the acquisition direction. Might not be intentional or desirable.\nAlready have {total_filled:?}")
		}

		self.limit = adj.limit;
		self.size_q = size;
		self.side = side;
		Ok(())
	}

	/// Produces the set of exact target orders that we want to see currently outstanding.
	/// Returns None if the output is unchanged from the previous call.
	///
	/// no generics or "semantic" stuff at this level, - we produce exact limit orders for exact exchange with exact configuration
	pub async fn next(&mut self) -> Result<Option<Vec<ExchangeOrder<LimitOrder>>>, Error> {
		let book = self.__book.snapshot();

		//HACK: the dumbest Chase Limit imaginable
		//TODO: make proper Iceberg protocol
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
		let single_sad_chase_limit = ExchangeOrder::new(
			limit,
			v_exchanges::Ticker {
				symbol: self.symbol,
				exchange_name: ExchangeName::Bybit, //dbg
			},
		);
		let orders = vec![single_sad_chase_limit];

		if self.__prev == orders {
			return Ok(None);
		}
		self.__prev = orders.clone(); //HACK: uhhh, gotta be a better way with reuse of memory
		Ok(Some(orders))
	}

	pub(crate) fn from_args(v: ConceptualLimitArgs, book: BookRef) -> Self {
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
			__book: book,
			__filled: AHashMap::default(),
			__prev: Vec::default(),
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

impl Keyed for ConceptualLimit {
	type Key = Uuid;

	fn keys(&self) -> MyKey<Uuid> {
		MyKey::new(self.id, None) //TODO!!!: parent should link to the Protocol that requested it
	}
}

//TODO: move to v_exchanges
// want to have an object fully encompasing all the relevant physical updates
#[derive(Debug, derive_more::Display, thiserror::Error, derive_more::From)]
/// Error during the conversion of intent into exact orders
pub enum Error {
	Other(miette::Report),
}
