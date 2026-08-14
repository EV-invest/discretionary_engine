use std::{
	cell::UnsafeCell,
	pin::Pin,
	sync::Arc,
	time::{SystemTime, UNIX_EPOCH},
};

use ahash::AHashMap;
use arc_swap::ArcSwap;
use exchange_interactions::{ExchangeName, ExchangeStream, Instrument, adapters::generics::ws::WsError};
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use tokio::sync::Notify;
use trading_data_core::{Asset, BookUpdate, Exact, Local, PrecisionPriceQty, Price, ShadowBook, Ts};

/// Never read — we take no checkpoints here, persistence is not this crate's job — but
/// [`ShadowBook`] needs one to be constructed.
const CHECKPOINT_CADENCE: Exact = Exact::from_nanos(60_000_000_000);
/// Shared read handle to a continuously-updated top of book. Cheaply cloneable.
pub type BookRef = Arc<BookShared>;

/// A feed is one (venue, instrument): spot and perp are separate books, and folding one's deltas
/// into the other's ladder would corrupt both.
type FeedId = (ExchangeName, Instrument);
type PullResult = (FeedId, Box<dyn ExchangeStream<Item = BookUpdate>>, Result<Vec<BookUpdate>, WsError>);
/// Best bid/ask across every subscribed feed.
///
/// Deliberately not a merged ladder: [`trading_data_core::BookShape`] carries one
/// [`PrecisionPriceQty`], and each venue reports on its own tick, so raw levels from two feeds are
/// not the same unit and cannot share a map. [`Price`] carries its precision with it and orders by
/// value, which is what makes the comparison below meaningful across venues.
#[derive(Clone, Copy, Debug, Default)]
pub struct Top {
	pub bid: Option<Price>,
	pub ask: Option<Price>,
}

pub struct BookShared {
	top: ArcSwap<Top>,
	notify: Notify,
}
impl BookShared {
	/// Current cross-venue top of book.
	pub fn top(&self) -> Arc<Top> {
		self.top.load_full()
	}

	/// Wait until the book has been updated.
	pub async fn tick(&self) {
		self.notify.notified().await;
	}
}

impl Default for BookShared {
	fn default() -> Self {
		Self {
			top: ArcSwap::from_pointee(Top::default()),
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
	pub async fn new(asset: Asset) -> Self {
		let mut inner = BookInner {
			feeds: AHashMap::default(),
			futs: FuturesUnordered::default(),
		};

		let mut exchanges = de_core::config::build_exchanges();
		for (exch, instruments) in &mut exchanges {
			let name = exch.name();
			let Some(feed) = exch.stream() else {
				tracing::warn!(exchange = %name, asset = %asset, "no live feed");
				continue;
			};
			for instrument in instruments.iter().copied() {
				let pair = asset.usd_pair(instrument == Instrument::PerpInverse);
				match feed.ws_book(&[pair], instrument).await {
					Ok(mut stream) => {
						tracing::info!(exchange = %name, asset = %asset, %instrument, "subscribed to ws_book");
						let id = (name, instrument);
						// precision is seeded from the feed's first message
						inner.feeds.insert(id, ShadowBook::new(PrecisionPriceQty::default(), CHECKPOINT_CADENCE));
						inner.futs.push(Box::pin(async move {
							let result = stream.next().await;
							(id, stream, result)
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

	/// Pull one batch from any feed, fold and publish.
	/// SAFETY: caller must ensure this is only called from a single task.
	pub async fn pull(&self) {
		// SAFETY: only the DataHub poll loop calls this, single-threaded access guaranteed.
		let inner = unsafe { &mut *self.inner.get() };

		let Some((id, stream, result)) = inner.futs.next().await else {
			std::future::pending::<()>().await;
			unreachable!();
		};

		match result {
			Ok(updates) => {
				let recv = now();
				let shadow = inner.feeds.get_mut(&id).expect("inserted on subscribe, dropped only along with its stream");
				for u in &updates {
					// the emitted rows are for persistence; here we only read the fold they were applied to
					let _ = shadow.ingest(u, recv);
				}
				self.publish(inner);

				let mut stream = stream;
				inner.futs.push(Box::pin(async move {
					let result = stream.next().await;
					(id, stream, result)
				}));
			}
			Err(e) => {
				tracing::error!(exchange = %id.0, instrument = %id.1, "ws_book error: {e}");
				inner.feeds.remove(&id);
			}
		}
	}

	fn publish(&self, inner: &BookInner) {
		let mut top = Top::default();
		for shadow in inner.feeds.values() {
			let book = shadow.book();
			if let Some((bid, _)) = book.best_bid() {
				top.bid = Some(top.bid.map_or(bid, |best| best.max(bid)));
			}
			if let Some((ask, _)) = book.best_ask() {
				top.ask = Some(top.ask.map_or(ask, |best| best.min(ask)));
			}
		}
		self.shared.top.store(Arc::new(top));
		self.shared.notify.notify_waiters();
	}
}

struct BookInner {
	feeds: AHashMap<FeedId, ShadowBook>,
	futs: FuturesUnordered<Pin<Box<dyn std::future::Future<Output = PullResult> + Send>>>,
	//TODO!!!: history
}

/// SAFETY: only the single DataHub poll task ever accesses the UnsafeCell contents.
unsafe impl Sync for Book {}

fn now() -> Ts<Local> {
	let since_epoch = SystemTime::now().duration_since(UNIX_EPOCH).expect("system clock is after the unix epoch");
	Ts::from_nanos(since_epoch.as_nanos() as i64)
}
