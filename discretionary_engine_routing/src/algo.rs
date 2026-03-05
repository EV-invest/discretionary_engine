use miette::Result;
use uuid::Uuid;
use v_exchanges::Symbol;
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
}

#[derive(Clone, Debug)]
pub struct ConceptualLimit {
	limit: f32,
	symbol: Symbol,
	size_q: f32,
	pub id: Uuid,
	side: Side,
}

#[derive(thiserror::Error, Debug, derive_more::From, derive_more::Display)]
/// Error during the conversion of intent into exact orders
pub enum Error {
	Other(miette::Report),
}

impl ConceptualLimit {
	pub fn next(&self) -> Result<(), Error> {
		//HACK: should be a proper custom error type
		todo!()
	}
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
