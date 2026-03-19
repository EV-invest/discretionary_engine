use std::{collections::HashMap, sync::Arc};

use miette::Result;
use uuid::Uuid;
use v_exchanges::{BookDelta, BookSnapshot, ExchangeName, ExchangeOrder, Symbol, Trade, orders::LimitOrder};
use v_utils::trades::Side;

#[derive(clap::Args, Debug)]
pub struct ConceptualLimitArgs {
	/// follows rules for normal limit orders
	#[arg(long)]
	pub limit: f64,

	/// gimme [Symbol](v_exchanges::Symbol)
	#[arg(long)]
	pub symbol: Symbol,

	/// qty size (signed, - side inferred)
	#[arg(long)]
	pub qty: f64,
	//TODO: the actually juicy parts like the relative cost of price diff vs time
}

#[derive(Clone, Debug, derive_new::new)]
pub struct ConceptualLimit {
	pub id: Uuid,

	limit: f64,
	symbol: Symbol,
	size_q: f64,
	side: Side,

	/// total per-exchange qty fill value
	__filled: HashMap<ExchangeName, f32>,
	__book: Arc<v_exchanges::Book>,
}
impl ConceptualLimit {
	/// produces the vec of exact target orders that we want to see currently outstanding
	///
	/// no generics or "semantic" stuff at this level, - we produce exact limit orders for exact exchange with exact configuration  
	pub async fn next(&self, diff: BookUpdate) -> Result<Vec<ExchangeOrder<LimitOrder>>, Error> {
		//dbg
		let limit = LimitOrder::new(self.side, self.limit, self.size_q);
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
}

//TODO: move to v_exchanges
// want to have an object fully encompasing all the relevant physical updates
#[derive(Debug, derive_more::Display, thiserror::Error, derive_more::From)]
/// Error during the conversion of intent into exact orders
pub enum Error {
	Other(miette::Report),
}
struct BookUpdate {
	book: BookDelta, // might want to have separate (book, tape) for each exchange
	tape: Vec<Trade>,
}

impl From<ConceptualLimitArgs> for ConceptualLimit {
	fn from(v: ConceptualLimitArgs) -> Self {
		let (size, side) = match v.qty {
			p if p > 0. => (p, Side::Buy),
			p if p < 0. => (-p, Side::Sell),
			_ => unreachable!("should've checked before here, - where we still can report to user"),
		};
		ConceptualLimit {
			side,
			limit: v.limit,
			size_q: size,
			symbol: v.symbol,

			id: Uuid::now_v7(),
			__filled: HashMap::new(),
			__book: Arc::default(),
		}
	}
}
