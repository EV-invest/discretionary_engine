use std::{collections::HashMap, sync::Arc};

use tokio::{sync::Notify, task::JoinHandle};
use v_exchanges::{BookShape, BookUpdate, ExchangeName, ExchangeStream, Instrument};
use v_utils::trades::Asset;

/// Shared handle to a Book's state — cloneable, Send + Sync.
/// Held by ConceptualLimits to read orderbook data.
#[derive(Clone, Debug)]
pub struct BookHandle {
	state: Arc<parking_lot::RwLock<BookShape>>,
	notify: Arc<Notify>,
}
impl BookHandle {
	/// Wait until the book has been updated. Returns nothing — read state via `snapshot()`.
	pub async fn tick(&self) {
		self.notify.notified().await;
	}

	/// Current merged orderbook snapshot.
	pub fn snapshot(&self) -> BookShape {
		self.state.read().clone()
	}
}

/// Per-asset aggregated orderbook.
///
/// Spawns a background task that polls ws_book streams from all configured exchanges
/// and maintains merged orderbook state. The task is aborted when Book is dropped.
pub struct Book {
	asset: Asset,
	handle: BookHandle,
	_task: JoinHandle<()>,
}
impl Book {
	/// Create a new Book for the given asset and subscribe to ws_book on all configured exchanges.
	/// Spawns a background task that continuously polls streams.
	pub fn new(asset: Asset) -> Self {
		let exchanges = de_core::config::build_exchanges();

		let state: Arc<parking_lot::RwLock<BookShape>> = Arc::default();
		let notify = Arc::new(Notify::new());
		let handle = BookHandle {
			state: Arc::clone(&state),
			notify: Arc::clone(&notify),
		};

		let mut streams: Vec<(ExchangeName, Box<dyn ExchangeStream<Item = BookUpdate>>)> = Vec::new();
		for (exch, instruments) in &exchanges {
			let name = exch.name();
			for &instrument in instruments {
				let pair = asset.usd_pair(instrument == Instrument::PerpInverse);
				match exch.ws_book(vec![pair], instrument) {
					Ok(stream) => {
						tracing::info!(exchange = %name, asset = %asset, %instrument, "subscribed to ws_book");
						streams.push((name.clone(), stream));
					}
					Err(e) => {
						tracing::warn!(exchange = %name, asset = %asset, %instrument, "ws_book not supported: {e}");
					}
				}
			}
		}

		let task = tokio::spawn(async move {
			Self::poll_loop(asset, streams, state, notify).await;
		});

		Self { asset, handle, _task: task }
	}

	/// Get a cloneable handle for ConceptualLimits to hold.
	pub fn handle(&self) -> BookHandle {
		self.handle.clone()
	}

	/// Background polling loop. Uses FuturesUnordered with ownership transfer (btc_line pattern)
	/// to avoid cancellation of in-flight stream reads.
	async fn poll_loop(asset: Asset, streams: Vec<(ExchangeName, Box<dyn ExchangeStream<Item = BookUpdate>>)>, state: Arc<parking_lot::RwLock<BookShape>>, notify: Arc<Notify>) {
		use std::pin::Pin;

		use futures_util::{StreamExt as _, stream::FuturesUnordered};

		let mut exchange_books: HashMap<ExchangeName, ExchangeBook> = streams.iter().map(|(name, _)| (name.clone(), ExchangeBook::default())).collect();

		type StreamItem = (ExchangeName, Box<dyn ExchangeStream<Item = BookUpdate>>);
		type PollResult = (StreamItem, Result<BookUpdate, v_exchanges::adapters::generics::ws::WsError>);
		let mut futs: FuturesUnordered<Pin<Box<dyn std::future::Future<Output = PollResult> + Send>>> = FuturesUnordered::new();

		// Seed all streams into FuturesUnordered
		for (name, stream) in streams {
			futs.push(Box::pin(async move {
				let mut stream = stream;
				let result = stream.next().await;
				((name, stream), result)
			}));
		}

		while let Some(((name, stream), result)) = futs.next().await {
			match result {
				Ok(update) => {
					if let Some(ebook) = exchange_books.get_mut(&name) {
						ebook.apply(update);
					}
					Self::publish_merged(&exchange_books, &state);
					notify.notify_waiters();

					// Re-schedule this stream
					futs.push(Box::pin(async move {
						let mut stream = stream;
						let result = stream.next().await;
						((name, stream), result)
					}));
				}
				Err(e) => {
					tracing::error!(exchange = %name, asset = %asset, "ws_book error: {e}");
					exchange_books.remove(&name);
					// Don't re-schedule — stream is dead
				}
			}
		}

		tracing::warn!(asset = %asset, "Book has no streams left, task exiting");
	}

	/// Merge all per-exchange books into a single BookShape and write to shared state.
	fn publish_merged(exchange_books: &HashMap<ExchangeName, ExchangeBook>, state: &parking_lot::RwLock<BookShape>) {
		let mut merged = BookShape::default();
		for ebook in exchange_books.values() {
			merged.asks.extend_from_slice(&ebook.shape.asks);
			merged.bids.extend_from_slice(&ebook.shape.bids);
			if ebook.shape.time > merged.time {
				merged.time = ebook.shape.time;
			}
		}
		// Sort asks ascending, bids descending
		merged.asks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
		merged.bids.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
		*state.write() = merged;
	}
}

/// Per-exchange orderbook state, applying snapshots and deltas.
#[derive(Clone, Debug, Default)]
struct ExchangeBook {
	shape: BookShape,
}
impl ExchangeBook {
	fn apply(&mut self, update: BookUpdate) {
		match update {
			BookUpdate::Snapshot(shape) => {
				self.shape = shape;
			}
			BookUpdate::Delta(delta) => {
				self.shape.time = delta.time;
				Self::apply_side(&mut self.shape.bids, &delta.bids);
				Self::apply_side(&mut self.shape.asks, &delta.asks);
			}
		}
	}

	/// Apply delta levels: qty=0 removes, otherwise upserts.
	fn apply_side(levels: &mut Vec<(f64, f64)>, deltas: &[(f64, f64)]) {
		for &(price, qty) in deltas {
			if qty == 0.0 {
				levels.retain(|&(p, _)| (p - price).abs() > f64::EPSILON);
			} else if let Some(existing) = levels.iter_mut().find(|(p, _)| (*p - price).abs() <= f64::EPSILON) {
				existing.1 = qty;
			} else {
				levels.push((price, qty));
			}
		}
	}
}

impl std::fmt::Debug for Book {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("Book").field("asset", &self.asset).finish()
	}
}
impl Drop for Book {
	fn drop(&mut self) {
		self._task.abort();
	}
}
