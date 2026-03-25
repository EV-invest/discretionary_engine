mod book;

use std::{collections::HashMap, sync::OnceLock};

use tokio::sync::mpsc;
use v_utils::trades::Asset;

pub use crate::book::{BookRef, BookShared};

/// Get or create a BookRef for the given asset.
pub async fn book(asset: Asset) -> BookRef {
	let hub = DATA_HUB.get_or_init(|| {
		let (tx, rx) = mpsc::unbounded_channel();
		tokio::spawn(poll_loop(rx));
		DataHub { req_tx: tx }
	});

	let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
	hub.req_tx.send(BookRequest { asset, reply: reply_tx }).expect("DataHub poll loop dead");
	reply_rx.await.expect("DataHub poll loop dead")
}
static DATA_HUB: OnceLock<DataHub> = OnceLock::new();

struct DataHub {
	req_tx: mpsc::UnboundedSender<BookRequest>,
}

struct BookRequest {
	asset: Asset,
	reply: tokio::sync::oneshot::Sender<BookRef>,
}

async fn poll_loop(mut req_rx: mpsc::UnboundedReceiver<BookRequest>) {
	let mut books: HashMap<Asset, book::Book> = HashMap::new();
	let mut pending: Vec<BookRequest> = Vec::new();

	loop {
		// Apply pending requests
		for req in pending.drain(..) {
			let shared = books.entry(req.asset).or_insert_with(|| book::Book::new(req.asset)).shared.clone();
			let _ = req.reply.send(shared);
		}

		// Drain channel
		while let Ok(req) = req_rx.try_recv() {
			let shared = books.entry(req.asset).or_insert_with(|| book::Book::new(req.asset)).shared.clone();
			let _ = req.reply.send(shared);
		}

		if books.is_empty() {
			if let Some(req) = req_rx.recv().await {
				let b = book::Book::new(req.asset);
				let _ = req.reply.send(b.shared.clone());
				books.insert(req.asset, b);
			} else {
				break;
			}
			continue;
		}

		// Poll all books concurrently
		let mut futs = futures_util::stream::FuturesUnordered::new();
		for book in books.values() {
			futs.push(book.pull());
		}

		use futures_util::StreamExt as _;
		tokio::select! {
			biased;
			Some(req) = req_rx.recv() => {
				pending.push(req);
			}
			_ = futs.next() => {}
		}
		// futs dropped here — books borrow released before next iteration
	}
}
