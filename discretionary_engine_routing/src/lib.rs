pub mod algo;
pub mod data;

use std::collections::HashMap;

use color_eyre::eyre::Result;
use de_core::component::{Component, ComponentState, ComponentTrigger};
use tracing::info;
use uuid::Uuid;

use crate::algo::{ConceptualLimit, ConceptualLimitChangeable};

pub const STREAM_KEY: &str = "discretionary_engine:routing:commands";
pub const CONSUMER_GROUP: &str = "routing_consumers";

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

#[derive(Debug)]
pub struct RoutingHub {
	conceptual_limits: HashMap<Uuid, ConceptualLimit>,
	state: ComponentState,
}
impl RoutingHub {
	pub fn new() -> Self {
		let mut hub = Self {
			conceptual_limits: HashMap::new(),
			state: ComponentState::default(),
		};
		hub.transition_state(ComponentTrigger::Initialize);
		hub
	}

	/// Main runtime loop. Subscribes to Redis, processes commands, polls active limits.
	pub async fn run(&mut self, redis_port: u16) -> Result<()> {
		self.start();

		let consumer_name = format!("routing-{}", std::process::id());
		let mut conn = de_core::redis_bus::connect(redis_port).await?;
		let mut subscriber = de_core::redis_bus::StreamSubscriber::new(&mut conn, STREAM_KEY, CONSUMER_GROUP, consumer_name).await?;

		info!("RoutingHub running, listening for commands on Redis port {redis_port}...");

		loop {
			tokio::select! {
				result = subscriber.next::<Commands>() => {
					match result {
						Ok(Some(cmd)) => {
							self.handle_command(cmd);
						}
						Ok(None) => {
							// timeout, poll active limits
						}
						Err(e) => {
							tracing::error!("Error reading routing command: {e}");
						}
					}
				}
				_ = tokio::signal::ctrl_c() => {
					info!("RoutingHub shutting down...");
					break;
				}
			}

			self.poll_limits().await;
		}

		self.stop();
		Ok(())
	}

	fn handle_command(&mut self, cmd: Commands) {
		match cmd {
			Commands::New(args) => {
				let limit = ConceptualLimit::from(args);
				info!(id = %limit.id, "New ConceptualLimit added");
				self.conceptual_limits.insert(limit.id, limit);
			}
			Commands::Adj { id, args } =>
				if let Some(existing) = self.conceptual_limits.get_mut(&id) {
					match existing.adjust(args) {
						Ok(()) => info!(%id, "ConceptualLimit adjusted"),
						Err(e) => tracing::error!(%id, "Adj rejected: {e}"),
					}
				} else {
					tracing::warn!(%id, "Adj: ConceptualLimit not found");
				},
			Commands::Del { id } =>
				if self.conceptual_limits.remove(&id).is_some() {
					info!(%id, "ConceptualLimit removed");
				} else {
					tracing::warn!(%id, "Del: ConceptualLimit not found");
				},
		}
	}

	async fn poll_limits(&self) {
		for (id, limit) in &self.conceptual_limits {
			match limit.next().await {
				Ok(orders) =>
					for order in &orders {
						info!(%id, ?order, "ConceptualLimit produced order");
					},
				Err(e) => {
					tracing::error!(%id, "ConceptualLimit::next() failed: {e}");
				}
			}
		}
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
