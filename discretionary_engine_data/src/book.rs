use std::{cell::UnsafeCell, pin::Pin, sync::Arc};

use ahash::AHashMap;
use arc_swap::ArcSwap;
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use tokio::sync::Notify;
use v_exchanges::{BookShape, BookUpdate, ExchangeName, ExchangeStream};
use v_utils::trades::Asset;

/// Shared read handle to a continuously-updated orderbook. Cheaply cloneable.
pub type BookRef = Arc<BookShared>;

type PullResult = (
	ExchangeName,
	Box<dyn ExchangeStream<Item = BookUpdate>>,
	Result<BookUpdate, v_exchanges::adapters::generics::ws::WsError>,
);
pub struct BookShared {
	snapshot: ArcSwap<BookShape>,
	notify: Notify,
}
impl BookShared {
	/// Current merged orderbook snapshot.
	pub fn snapshot(&self) -> Arc<BookShape> {
		self.snapshot.load_full()
	}

	/// Wait until the book has been updated.
	pub async fn tick(&self) {
		self.notify.notified().await;
	}
}

impl Default for BookShared {
	fn default() -> Self {
		Self {
			snapshot: ArcSwap::from_pointee(BookShape::default()),
			notify: Notify::new(),
		}
	}
}

impl std::fmt::Debug for BookShared {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("BookShared").finish_non_exhaustive()
	}
}

/// Per-asset orderbook. Owned by DataHub, not shared directly.
/// Interior mutability via UnsafeCell — only the single poll loop ever calls `pull()`.
pub(crate) struct Book {
	pub shared: BookRef,
	inner: UnsafeCell<BookInner>,
}
impl Book {
	pub fn new(asset: Asset) -> Self {
		let mut inner = BookInner {
			exchange_shapes: AHashMap::default(),
			futs: FuturesUnordered::default(),
		};

		let exchanges = de_core::config::build_exchanges();
		for (exch, instruments) in &exchanges {
			let name = exch.name();
			for instrument in instruments {
				let pair = asset.usd_pair(*instrument == v_exchanges::Instrument::PerpInverse);
				match exch.ws_book(vec![pair], *instrument) {
					Ok(stream) => {
						tracing::info!(exchange = %name, asset = %asset, %instrument, "subscribed to ws_book");
						inner.exchange_shapes.insert(name.clone(), BookShape::default());
						let name = name.clone();
						let mut stream = stream;
						inner.futs.push(Box::pin(async move {
							let result = stream.next().await;
							(name, stream, result)
						}));
					}
					Err(e) => {
						tracing::warn!(exchange = %name, asset = %asset, %instrument, "ws_book not supported: {e}");
					}
				}
			}
		}

		Self {
			shared: Arc::default(),
			inner: UnsafeCell::new(inner),
		}
	}

	/// Pull one update from any exchange stream, apply and publish.
	/// SAFETY: caller must ensure this is only called from a single task.
	pub async fn pull(&self) {
		// SAFETY: only the DataHub poll loop calls this, single-threaded access guaranteed.
		let inner = unsafe { &mut *self.inner.get() };

		let Some((name, stream, result)) = inner.futs.next().await else {
			std::future::pending::<()>().await;
			unreachable!();
		};

		match result {
			Ok(update) => {
				if let Some(shape) = inner.exchange_shapes.get_mut(&name) {
					match update {
						BookUpdate::Snapshot(s) => *shape = s,
						BookUpdate::Delta(delta) => {
							shape.time = delta.time;
							apply_side(&mut shape.bids, &delta.bids);
							apply_side(&mut shape.asks, &delta.asks);
						}
					}
				}
				self.publish_merged(inner);

				let mut stream = stream;
				inner.futs.push(Box::pin(async move {
					let result = stream.next().await;
					(name, stream, result)
				}));
			}
			Err(e) => {
				tracing::error!(exchange = %name, "ws_book error: {e}");
				inner.exchange_shapes.remove(&name);
			}
		}
	}

	fn publish_merged(&self, inner: &BookInner) {
		let mut merged = BookShape::default();
		for shape in inner.exchange_shapes.values() {
			merged.asks.extend_from_slice(&shape.asks);
			merged.bids.extend_from_slice(&shape.bids);
			if shape.time > merged.time {
				merged.time = shape.time;
			}
		}
		merged.asks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
		merged.bids.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
		self.shared.snapshot.store(Arc::new(merged));
		self.shared.notify.notify_waiters();
	}
}

struct BookInner {
	exchange_shapes: AHashMap<ExchangeName, BookShape>,
	futs: FuturesUnordered<Pin<Box<dyn std::future::Future<Output = PullResult> + Send>>>,
	//TODO!!!: history
}

/// SAFETY: only the single DataHub poll task ever accesses the UnsafeCell contents.
unsafe impl Sync for Book {}

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
