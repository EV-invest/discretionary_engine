pub mod algo;
pub mod data;

use std::{collections::HashMap, pin::Pin};

use color_eyre::eyre::Result;
use de_core::component::{Component, ComponentState, ComponentTrigger};
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use tracing::info;
use uuid::Uuid;
use v_exchanges::{ExchangeOrder, orders::LimitOrder};
use v_utils::trades::Asset;

use crate::{
	algo::{ConceptualLimit, ConceptualLimitChangeable},
	data::Book,
};

pub const STREAM_KEY: &str = "discretionary_engine:routing:commands";
pub const CONSUMER_GROUP: &str = "routing_consumers";

type LimitResult = (Uuid, ConceptualLimit, std::result::Result<Vec<ExchangeOrder<LimitOrder>>, algo::Error>);
type LimitFut = Pin<Box<dyn std::future::Future<Output = LimitResult> + Send>>;
#[derive(Debug, serde::Deserialize, serde::Serialize, clap::Subcommand)]
pub enum Commands {
	New(algo::ConceptualLimitArgs),
	//Q: my current understanding is Adj and Del can attempt to directly change the outstanding ConceptualLimit protocols. Since they are exclusively pulled, and also at this level we do not generate new opinions on delta intent, we can just instantly accept whatever suggestion inputted
	//A: thus these will just directly change values on list of ConceptualLimits we're listening on atm
	Adj {
		#[arg(long)]
		id: Uuid,
		#[command(flatten)]
		args: ConceptualLimitChangeable,
	},
	Del {
		#[arg(long)]
		id: Uuid,
	},
}

/// Client-side: parse CLI command, serialize, publish to Redis, exit.
pub async fn publish(cmd: Commands, redis_port: u16) -> Result<()> {
	let mut conn = de_core::redis_bus::connect(redis_port).await?;
	let id = de_core::redis_bus::publish(&mut conn, STREAM_KEY, &cmd).await?;
	info!("Routing command published with ID: {id}");
	Ok(())
}

pub struct RoutingHub {
	books: HashMap<Asset, Book>,
	limit_futs: FuturesUnordered<LimitFut>,
	deleted: std::collections::HashSet<Uuid>,
	state: ComponentState,
}
impl RoutingHub {
	#[deprecated(note = "think we can get rid of `new` entirely and switch to derive Default")]
	pub fn new() -> Self {
		let mut hub = Self {
			books: HashMap::new(),
			limit_futs: FuturesUnordered::new(),
			deleted: std::collections::HashSet::new(),
			state: ComponentState::default(),
		};
		hub.transition_state(ComponentTrigger::Initialize);
		hub
	}

	pub fn handle_command(&mut self, cmd: Commands) {
		match cmd {
			Commands::New(args) => {
				let asset = *args.symbol.pair.base();

				if !self.books.contains_key(&asset) {
					self.spawn_book(asset);
				}

				let book_handle = self.books.get(&asset).expect("just inserted").handle();
				let limit = ConceptualLimit::from_args(args, book_handle);
				let id = limit.id;
				info!(%id, "New ConceptualLimit added");
				self.limit_futs.push(Self::make_limit_fut(id, limit));
			}
			Commands::Adj { id, args } => {
				//TODO: apply adjustment immediately (requires pulling limit out of FuturesUnordered)
				tracing::warn!(%id, ?args, "Adj received — will apply on next cycle (not yet implemented for in-flight limits)");
			}
			Commands::Del { id } => {
				self.deleted.insert(id);
				info!(%id, "ConceptualLimit marked for deletion");
			}
		}
	}

	/// Blocks until any ConceptualLimit produces orders. Returns the id and orders.
	///
	/// Books take care of themselves (internal tokio::spawn'd tasks, killed on drop).
	/// Limits are polled via FuturesUnordered (btc_line pattern): each limit is moved
	/// into a future that blocks on `book.tick()` then produces orders. On completion
	/// the limit is returned and re-scheduled.
	pub async fn next(&mut self) -> (Uuid, std::result::Result<Vec<ExchangeOrder<LimitOrder>>, algo::Error>) {
		loop {
			let (id, limit, result) = self.limit_futs.next().await.expect("RoutingHub::next() called with no active limits");
			if self.deleted.remove(&id) {
				continue;
			}
			// Re-schedule
			self.limit_futs.push(Self::make_limit_fut(id, limit));
			return (id, result);
		}
	}

	fn spawn_book(&mut self, asset: Asset) {
		info!(asset = %asset, "Spawning Book");
		let book = Book::new(asset);
		self.books.insert(asset, book);
	}

	fn make_limit_fut(id: Uuid, limit: ConceptualLimit) -> LimitFut {
		Box::pin(async move {
			let result = limit.next().await;
			(id, limit, result)
		})
	}
}

impl Default for RoutingHub {
	fn default() -> Self {
		Self::new()
	}
}

impl std::fmt::Debug for RoutingHub {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RoutingHub").field("n_books", &self.books.len()).field("state", &self.state).finish()
	}
}

impl Component for RoutingHub {
	fn state(&self) -> ComponentState {
		self.state
	}

	fn transition_state(&mut self, trigger: ComponentTrigger) {
		self.state.transition(trigger);
	}
}

//DO: want some kind of tracking system for all `ConceptualLimit`s in action
//DO: and then change/remove will naturally integrate with it
//Nb: at this level there is no interpreting and selecting from orders generated from ConceptualLimit processes, - we just take and execute them as-is. Thinking about what others are doing is on `_strategy`, - in here we just do what we're told

#[derive(Debug, derive_more::Display, derive_more::From)]
pub enum RoutingError {
	Invalid(InvalidRoutingError),
	Other(miette::Error),
}

#[derive(Debug, derive_more::Display)]
pub enum InvalidRoutingError {
	#[display("adjustment would reverse position (hint: submit a Del and a new New instead)")]
	AdjustmentWouldReverse,
}
