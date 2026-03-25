pub mod algo;
pub mod data;

use std::{
	collections::{HashMap, HashSet},
	pin::Pin,
};

use de_core::component::{Component, ComponentState, ComponentTrigger};
use futures_util::StreamExt as _;
use tokio_stream::StreamMap;
use tracing::info;
use uuid::Uuid;
use v_exchanges::{ExchangeOrder, orders::LimitOrder};
use v_utils::trades::Asset;

use crate::{algo::ConceptualLimit, data::Book};

pub const STREAM_KEY: &str = "discretionary_engine:routing:commands";
pub const CONSUMER_GROUP: &str = "routing_consumers";

pub type LimitStepResult = (Uuid, std::result::Result<Vec<ExchangeOrder<LimitOrder>>, algo::Error>);

#[derive(Debug, serde::Deserialize, serde::Serialize, clap::Subcommand)]
pub enum Commands {
	New(algo::ConceptualLimitArgs),
	Adj {
		#[arg(long)]
		id: Uuid,
		#[command(flatten)]
		args: algo::ConceptualLimitChangeable,
	},
	Del {
		#[arg(long)]
		id: Uuid,
		/// When true, the delete is skipped if the queue contains a later Adj for the same id.
		/// Used when a ConceptualLimit self-deletes on completion but an adjustment may already be in flight.
		#[arg(long, default_value_t = false)]
		weak: bool,
	},
}

/// Client-side: parse CLI command, serialize, publish to Redis, exit.
pub async fn publish(cmd: Commands, redis_port: u16) -> color_eyre::eyre::Result<()> {
	let mut conn = de_core::redis_bus::connect(redis_port).await?;
	let id = de_core::redis_bus::publish(&mut conn, STREAM_KEY, &cmd).await?;
	info!("Routing command published with ID: {id}");
	Ok(())
}

type AssetStream = Pin<Box<dyn futures_util::Stream<Item = Vec<LimitStepResult>> + Send>>;

pub struct RoutingHub {
	assets: HashMap<Asset, (HashSet<ConceptualLimit>, Book)>,
	streams: StreamMap<Asset, AssetStream>,
	command_queue: Vec<Commands>,
	state: ComponentState,
}

impl RoutingHub {
	pub fn new() -> Self {
		let mut hub = Self {
			assets: HashMap::new(),
			streams: StreamMap::new(),
			command_queue: Vec::new(),
			state: ComponentState::default(),
		};
		hub.transition_state(ComponentTrigger::Initialize);
		hub
	}

	/// Queues a command to be applied on the next `next()` call.
	pub fn handle_command(&mut self, cmd: Commands) {
		self.command_queue.push(cmd);
	}

	/// Awaits until any asset's limits produce results after a book tick.
	/// Returns the asset and all limit results for that tick.
	pub async fn next(&mut self) -> (Asset, Vec<LimitStepResult>) {
		self.apply_commands();
		self.streams.next().await.expect("RoutingHub::next() called with no active limits")
	}

	fn apply_commands(&mut self) {
		let commands = std::mem::take(&mut self.command_queue);
		let mut dirty: HashSet<Asset> = HashSet::new();

		for (i, cmd) in commands.iter().enumerate() {
			match cmd {
				Commands::New(args) => {
					let asset = *args.symbol.pair.base();

					if !self.assets.contains_key(&asset) {
						info!(asset = %asset, "Spawning Book");
						self.assets.insert(asset, (HashSet::new(), Book::new(asset)));
					}

					let (limits, book) = self.assets.get_mut(&asset).expect("just inserted");
					let book_handle = book.handle();
					let limit = ConceptualLimit::from_args(args.clone(), book_handle);
					info!(id = %limit.id, "New ConceptualLimit added");
					limits.insert(limit);
					dirty.insert(asset);
				}
				Commands::Adj { id, args } =>
					for (asset, (limits, _)) in &mut self.assets {
						if let Some(mut limit) = limits.take_by_id(*id) {
							match limit.adjust(args.clone()) {
								Ok(()) => info!(%id, "ConceptualLimit adjusted"),
								Err(e) => tracing::error!(%id, "adjustment failed: {e}"),
							}
							limits.insert(limit);
							dirty.insert(*asset);
							break;
						}
					},
				Commands::Del { id, weak } => {
					if *weak {
						let dominated = commands[i + 1..].iter().any(|later| matches!(later, Commands::Adj { id: adj_id, .. } if adj_id == id));
						if dominated {
							tracing::debug!(%id, "weak Del skipped — later Adj exists in queue");
							continue;
						}
					}

					for (asset, (limits, _)) in &mut self.assets {
						if limits.remove_by_id(*id) {
							info!(%id, "ConceptualLimit deleted");
							dirty.insert(*asset);
							break;
						}
					}
				}
			}
		}

		for asset in dirty {
			let Some((limits, _)) = self.assets.get(&asset) else {
				continue;
			};

			if limits.is_empty() {
				self.assets.remove(&asset);
				self.streams.remove(&asset);
				info!(asset = %asset, "No limits remaining — Book dropped");
				continue;
			}

			self.rebuild_asset_stream(asset);
		}
	}

	fn rebuild_asset_stream(&mut self, asset: Asset) {
		let (limits, book) = self.assets.get(&asset).expect("called for existing asset");
		let book_handle = book.handle();
		let cloned_limits: Vec<ConceptualLimit> = limits.iter().cloned().collect();

		let stream = futures_util::stream::unfold((book_handle, cloned_limits), |(bh, limits)| async move {
			bh.tick().await;

			let mut results: Vec<LimitStepResult> = Vec::with_capacity(limits.len());
			for limit in &limits {
				let result = limit.next().await;
				results.push((limit.id, result));
			}
			Some((results, (bh, limits)))
		});

		self.streams.insert(asset, Box::pin(stream));
	}
}

impl Default for RoutingHub {
	fn default() -> Self {
		Self::new()
	}
}

impl std::fmt::Debug for RoutingHub {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RoutingHub").field("n_assets", &self.assets.len()).field("state", &self.state).finish()
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

//Nb: at this level there is no interpreting and selecting from orders generated from ConceptualLimit processes, - we just take and execute them as-is. Thinking about what others are doing is on `_strategy`, - in here we just do what we're told

/// Helper trait for `HashSet<ConceptualLimit>` lookup by Uuid.
/// Since `ConceptualLimit` eq/hash is by id, we iterate (sets are small, writes are rare).
trait LimitSetExt {
	fn take_by_id(&mut self, id: Uuid) -> Option<ConceptualLimit>;
	fn remove_by_id(&mut self, id: Uuid) -> bool;
}
impl LimitSetExt for HashSet<ConceptualLimit> {
	fn take_by_id(&mut self, id: Uuid) -> Option<ConceptualLimit> {
		let limit = self.iter().find(|l| l.id == id).cloned()?;
		self.remove(&limit);
		Some(limit)
	}

	fn remove_by_id(&mut self, id: Uuid) -> bool {
		let Some(limit) = self.iter().find(|l| l.id == id).cloned() else {
			return false;
		};
		self.remove(&limit)
	}
}

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
