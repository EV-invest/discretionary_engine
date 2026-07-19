#![feature(default_field_values)]
pub mod algo;
pub mod data;

use std::{pin::Pin, sync::Arc};

use ahash::{AHashMap, AHashSet};
use de_core::component::{Component, ComponentId, ComponentState, ComponentTrigger};
use de_exec::ExecOrder;
use futures_util::{StreamExt as _, stream::FuturesUnordered};
use miette::Result;
use tracing::info;
use uuid::Uuid;
use v_exchanges::{ExchangeOrder, orders::LimitOrder};
use v_utils::{arch::Keyed, trades::Asset};

use crate::algo::ConceptualLimit;

pub const STREAM_KEY: &str = "discretionary_engine:routing:commands";
pub const CONSUMER_GROUP: &str = "routing_consumers";

type AssetTick = Pin<Box<dyn std::future::Future<Output = Asset> + Send>>;
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
	let id = de_core::redis_bus::publish(&mut conn, STREAM_KEY, CONSUMER_GROUP, &cmd).await?;
	v_utils::log!("Routing command published with ID: {id}");
	Ok(())
}

#[derive(Debug, Default, derive_more::Deref, derive_more::DerefMut)]
/// Per-Asset Executor
///
/// Takes care of all the execution on a single asset
pub struct Executor {
	#[deref]
	#[deref_mut]
	inner: Vec<ConceptualLimit>,
	/// Live orders on the venue, keyed by parent CL uuid.
	/// ExecOrder::parent() == Some(cl_uuid)
	order_sink: AHashMap<Uuid, Vec<ExecOrder<LimitOrder>>>,
	/// Last known desired orders per CL — used when next() returns Ok(None)
	order_cache: AHashMap<Uuid, Arc<Vec<ExchangeOrder<LimitOrder>>>>,
}
impl Executor {
	pub async fn tick(&mut self) -> Result<(), ExecutorError> {
		// Iceberg
		{
			//DO: join await `next()` on all children, get back exact new desired target Vec<ExchangeOrder>
			let raw: Vec<(Uuid, std::result::Result<Option<Vec<ExchangeOrder<LimitOrder>>>, algo::Error>)> = futures_util::future::join_all(self.inner.iter_mut().map(|limit| {
				let id = limit.id();
				async move { (id, limit.next().await) }
			}))
			.await;

			let mut desired: AHashMap<Uuid, Arc<Vec<ExchangeOrder<LimitOrder>>>> = AHashMap::default();
			for (id, result) in raw {
				match result {
					Ok(Some(orders)) => {
						let arc = Arc::new(orders);
						self.order_cache.insert(id, Arc::clone(&arc));
						desired.insert(id, arc);
					}
					Ok(None) => {
						// use value from our cache
						if let Some(cached) = self.order_cache.get(&id) {
							desired.insert(id, Arc::clone(cached));
						}
					}
					Err(e) => tracing::error!(%id, "ConceptualLimit::next() failed: {e}"),
				}
			}

			//DO: for each, calculate (expected-impact-on-the-book / necessary_rate[^1])
			//[^1] to compare apple to apple, we reuse `necessary_rate` (min size/time to expect to fill). Think about it, - with all same, if one algo wants larger size than another, it's likely to be more important. So don't fight the implications trying to equal all out, - execute on user intent.
			//HACK: skipped the step; hardcoding expected_impact is 0 for all, no filtering //TODO!!!!: .
			//

			//DO: now look at the matching hashmap of our order_sink, and see if any are above/below quota.
			todo!();

			//DO: when taking a mask against existing, should have a grace premia for both exact price values, and unfilled size.

			//DO: in both cases, we randomly select orders to remove/add.
			// // won't lead to cache misses as we only do this on mismatch
			// // Also, don't forget to attach the Uuid of the parent CL when submitting
		}

		// Forcing
		{
			todo!();
		}

		Ok(())
	}
}

pub struct RoutingHub {
	assets: AHashMap<Asset, Executor>,
	listen_books: FuturesUnordered<AssetTick>,
	command_queue: Vec<Commands>,
	state: ComponentState,
}
impl RoutingHub {
	/// Awaits until any asset's book ticks, then drives that asset's executor.
	pub async fn next(&mut self) {
		// drain command queue
		self.apply_commands().await;

		// `CL`s don't make progress if book hasn't updated since last time. So this here is when all are done and we just sit and wait for any to be able to make progress when book updates again.
		if self.listen_books.is_empty() {
			std::future::pending::<()>().await;
			unreachable!()
		}
		let book_upd_on: Asset = self.listen_books.next().await.expect("FuturesUnordered yielded None despite non-empty set");

		//self.push_asset_tick(asset).await;
		//let executor = self.assets.get_mut(&asset).expect("ticked asset has no executor");
		////XXX: wtf. This makes zero sense. Instead just run all. The only things that needs to happen here is communication back to `Protocol`s about `ConceptualLimit`s progressing. That's literally it. And even that maybe could be just directly looked up by them.
		//if let Err(e) = executor.tick().await {
		//	tracing::error!(%asset, "Executor::tick() failed: {e:?}");
		//}

		todo!();
	}

	async fn apply_commands(&mut self) {
		let commands = std::mem::take(&mut self.command_queue);
		let mut dirty: AHashSet<Asset> = AHashSet::default();

		for (i, cmd) in commands.iter().enumerate() {
			match cmd {
				Commands::New(args) => {
					let asset = *args.symbol.pair.base();

					self.assets.entry(asset).or_insert_with(|| {
						info!(asset = %asset, "Initializing asset");
						Executor::default()
					});

					let book = de_data::book(asset).await;
					let limits = self.assets.get_mut(&asset).expect("just inserted");
					let limit = ConceptualLimit::from_args(args.clone(), book);
					info!(id = %limit.id, "New ConceptualLimit added");
					limits.push(limit);
					dirty.insert(asset);
				}
				Commands::Adj { id, args } =>
					for (asset, limits) in &mut self.assets {
						if let Some(mut limit) = limits.take_by_id(*id) {
							match limit.adjust(args.clone()) {
								Ok(()) => info!(%id, "ConceptualLimit adjusted"),
								Err(e) => tracing::error!(%id, "adjustment failed: {e}"),
							}
							limits.push(limit);
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

					for (asset, limits) in &mut self.assets {
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
			let Some(limits) = self.assets.get(&asset) else {
				continue;
			};

			if limits.is_empty() {
				self.assets.remove(&asset);
				info!(asset = %asset, "No limits remaining — asset removed");
				continue;
			}

			self.push_asset_tick(asset).await;
		}
	}

	/// alias for pushing to private `command_queue`
	pub fn push_command(&mut self, cmd: Commands) {
		self.command_queue.push(cmd);
	}

	#[deprecated(
		note = "this makes no sense. Reimplement from zero. Target usability: expose method to insert an Asset into shared DataHub, to updates from which we're listening directly (using some unsafe trickery)"
	)]
	async fn push_asset_tick(&mut self, asset: Asset) {
		let book = de_data::book(asset).await;
		self.listen_books.push(Box::pin(async move {
			book.tick().await;
			asset
		}));
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
/// Helper trait for `HashSet<ConceptualLimit>` lookup by Uuid.
/// Since `ConceptualLimit` eq/hash is by id, we iterate (sets are small, writes are rare).
trait LimitSetExt {
	fn take_by_id(&mut self, id: Uuid) -> Option<ConceptualLimit>;
	fn remove_by_id(&mut self, id: Uuid) -> bool;
}
#[derive(Debug)]
enum ExecutorError {}
impl Component for Executor {
	fn component_id(&self) -> ComponentId {
		todo!()
	}

	fn state(&self) -> ComponentState {
		todo!()
	}

	fn transition_state(&mut self, trigger: ComponentTrigger) {
		todo!()
	}
}

impl Default for RoutingHub {
	fn default() -> Self {
		let mut hub = Self {
			assets: AHashMap::default(),
			listen_books: FuturesUnordered::default(),
			command_queue: Vec::default(),
			state: ComponentState::default(),
		};
		hub.transition_state(ComponentTrigger::Initialize);
		hub
	}
}

impl std::fmt::Debug for RoutingHub {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.debug_struct("RoutingHub").field("n_assets", &self.assets.len()).field("state", &self.state).finish()
	}
}

impl Component for RoutingHub {
	fn component_id(&self) -> ComponentId {
		ComponentId::new(std::any::type_name::<Self>().split("::").last().unwrap())
	}

	fn state(&self) -> ComponentState {
		self.state
	}

	fn transition_state(&mut self, trigger: ComponentTrigger) {
		self.state.transition(trigger);
	}
}

//Nb: at this level there is no interpreting and selecting from orders generated from ConceptualLimit processes, - we just take and execute them as-is. Thinking about what others are doing is on `_strategy`, - in here we just do what we're told

impl LimitSetExt for Executor {
	fn take_by_id(&mut self, id: Uuid) -> Option<ConceptualLimit> {
		let pos = self.iter().position(|l| l.id == id)?;
		Some(self.swap_remove(pos))
	}

	fn remove_by_id(&mut self, id: Uuid) -> bool {
		let Some(pos) = self.iter().position(|l| l.id == id) else {
			return false;
		};
		self.swap_remove(pos);
		true
	}
}
