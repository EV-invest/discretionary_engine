use v_exchanges::{BookShape, ExchangeName, Trade};

#[derive(Debug)]
pub struct BookDelta {
	exch: ExchangeName,
	/// levels to be removed are sent with quantity = 0
	diff: BookShape,
}

/// contains high-level info containing history and current state of atomic market operations
#[derive(Debug, Default)]
pub struct Book {
	book: Option<BookDelta>, // might want to have separate (book, tape) for each exchange
	tape: Vec<Trade>,
}
impl Book {
	/// subscribe to Book or Tape websocket for any of the v_exchanges::Exchange
	pub fn subscribe(&self) -> miette::Result<()> {
		todo!();
	}

	/// wait for the next book state update, returning a snapshot of current book
	///
	/// Does not error, - invalid state would've been caught before here. And if book itself is fundamentally corrupted, we just abort.
	pub async fn tick(&self) -> BookShape {
		todo!();
	}
}
