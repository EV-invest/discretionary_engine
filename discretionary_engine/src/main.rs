#![allow(warnings)]
#![allow(clippy::comparison_to_empty)]
#![allow(clippy::get_first)]
#![allow(clippy::len_zero)] // wait, so are the ones in Cargo.toml not enough?
#![feature(trait_alias)]
#![feature(type_changing_struct_update)]
#![feature(stmt_expr_attributes)]
#![feature(error_generic_member_access)]

mod chase_limit;
pub mod config;
pub mod exchange_apis;
pub mod positions;
pub mod protocols;
mod risk;
mod shell_init;
pub mod utils;
mod ws_chase_limit;
use std::{
	sync::{Arc, atomic::AtomicU32},
	time::Duration,
};

use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Context, Result, bail};
use config::{LiveSettings, SettingsFlags};
use exchange_apis::{exchanges::Exchanges, hub, hub::PositionToHub};
use positions::*;
use tokio::{sync::mpsc, task::JoinSet};
use tracing::{info, instrument};
use v_utils::{
	log,
	trades::{Side, Timeframe},
	utils::exit_on_error,
};

pub static MUT_CURRENT_CONNECTION_FAILURES: AtomicU32 = AtomicU32::new(0);

pub static MAX_CONNECTION_FAILURES: u32 = 10;
#[derive(Parser)]
#[command(author, version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"), about, long_about = None)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
	#[command(flatten)]
	settings: SettingsFlags,
	#[arg(short, long, action = clap::ArgAction::SetTrue)]
	noconfirm: bool,
	/// Use testnet instead of mainnet
	#[arg(long, global = true)]
	testnet: bool,
}
#[derive(Subcommand)]
enum Commands {
	/// Start the main program
	Run(PositionArgs),
	/// Long-running service: initializes exchanges, starts RoutingHub, listens for commands
	Daemon,
	/// Risk management commands
	Risk {
		#[command(subcommand)]
		command: risk::RiskCommands,
	},
	/// Strategy commands (nautilus-based)
	Strategy {
		#[command(subcommand)]
		command: StrategyCommands,
	},
	/// Routing commands (fire-and-forget, publishes to running service via Redis)
	Routing {
		#[command(subcommand)]
		command: de_routing::Commands,
	},
	/// Shell aliases and completions. Usage: `discretionary_engine init <shell> | source`
	Init(shell_init::ShellInitArgs),
}
#[derive(Subcommand)]
enum StrategyCommands {
	/// Start the strategy listener
	Start,
	/// Submit a position request to the running strategy
	Submit(StrategySubmitArgs),
}
#[derive(Args, Clone, Debug)]
struct StrategySubmitArgs {
	/// Target change in exposure. So positive for buying, negative for selling.
	#[arg(short, long, allow_hyphen_values = true)]
	size_usdt: f64,
	/// _only_ the coin name itself. e.g. "BTC" or "ETH".
	#[arg(short, long)]
	coin: String,
	/// protocols parameters, in the format of "<protocol>-<params>", e.g. "ts:p0.5".
	#[arg(short, long)]
	protocols: Vec<String>,
}
#[derive(Args, Clone, Debug)]
struct PositionArgs {
	/// Target change in exposure. So positive for buying, negative for selling.
	#[arg(short, long)]
	size_usdt: f64,
	/// timeframe, in the format of "1m", "1h", "3M", etc.
	/// determines the target period for which we expect the edge to persist.
	#[arg(short, long)]
	tf: Option<Timeframe>,
	/// _only_ the coin name itself. e.g. "BTC" or "ETH". Providing full symbol currently will error on the stage of making price requests for the coin.
	#[arg(short, long)]
	coin: String,
	/// acquisition protocols parameters, in the format of "<protocol>-<params>", e.g. "ts:p0.5". Params consist of their starting letter followed by the value, e.g. "p0.5" for 0.5% offset. If multiple params are required, they are separated by '-'.
	#[arg(short, long)]
	acquisition_protocols: Vec<String>,
	/// followup protocols parameters, in the format of "<protocol>-<params>", e.g. "ts:p0.5". Params consist of their starting letter followed by the value, e.g. "p0.5" for 0.5% offset. If multiple params are required, they are separated by '-'.
	#[arg(short, long)]
	followup_protocols: Vec<String>,
}
#[tokio::main]
async fn main() -> Result<()> {
	let cli = Cli::parse();

	// Init doesn't require config
	if let Commands::Init(args) = cli.command {
		shell_init::output(args);
		return Ok(());
	}

	let live_settings = match LiveSettings::new(cli.settings, Duration::from_secs(5)) {
		Ok(ls) => Arc::new(ls),
		Err(e) => {
			eprintln!("Loading config failed: {e}");
			std::process::exit(1);
		}
	};

	// Handle risk/routing commands early - they don't need the full exchange infrastructure
	if let Commands::Risk { command } = cli.command {
		v_utils::clientside!(Some("risk"));
		exit_on_error(match command {
			risk::RiskCommands::Size(args) => risk::size_main(live_settings, args).await,
			risk::RiskCommands::Balance => risk::balance_main(live_settings).await,
		});
		return Ok(());
	}
	if let Commands::Routing { command } = cli.command {
		v_utils::clientside!(Some("routing"));
		let redis_port = live_settings.config()?.redis_port;
		exit_on_error(de_routing::publish(command, redis_port).await);
		return Ok(());
	}

	// Validate positions_dir exists
	let initial_config = live_settings.config()?;
	std::fs::create_dir_all(&initial_config.positions_dir).wrap_err_with(|| format!("Failed to create positions directory at {:?}", initial_config.positions_dir))?;
	// Create XDG state directory for logs and other state
	let state_dir = dirs::state_dir()
		.unwrap_or_else(|| dirs::home_dir().expect("Could not determine home directory").join(".local/state"))
		.join(config::EXE_NAME);
	std::fs::create_dir_all(&state_dir).wrap_err_with(|| format!("Failed to create state directory at {state_dir:?}"))?;
	v_utils::clientside!(Some("daemon"));
	let mut js = JoinSet::default();
	let exchanges_arc = Arc::new(
		Exchanges::init(live_settings.clone())
			.await
			.wrap_err_with(|| "Error initializing Exchanges, likely indicative of bad internet connection")?,
	);
	let tx = hub::init_hub(live_settings.clone(), &mut js, exchanges_arc.clone());

	exit_on_error(match cli.command {
		Commands::Run(args) => command_new(args, live_settings.clone(), tx, exchanges_arc).await,
		Commands::Daemon => {
			let config = live_settings.config()?;
			let redis_port = config.redis_port;

			de_core::config::init_exchanges(config.exchanges.clone());
			let mut routing_hub = de_routing::RoutingHub::default();

			let consumer_name = format!("routing-{}", std::process::id());
			let mut conn = de_core::redis_bus::connect(redis_port).await?;
			let mut subscriber = de_core::redis_bus::StreamSubscriber::try_new(&mut conn, de_routing::STREAM_KEY, de_routing::CONSUMER_GROUP, consumer_name).await?;

			info!("Service running, listening on Redis port {redis_port}...");

			loop {
				//dbg: not the place for it, - we shouldn't
				tokio::select! {
					(asset, results) = routing_hub.next() => {
						for (id, result) in results {
							match &result {
								Ok(Some(orders)) => {
									for order in orders {
										info!(%asset, %id, ?order, "produced order");
									}
								}
								Ok(None) => {}
								Err(e) => {
									tracing::error!(%asset, %id, "ConceptualLimit::next() failed: {e}");
								}
							}
						}
					}

					result = subscriber.next::<de_routing::Commands>() => {
						match result {
							Ok(Some(cmd)) => {
								tracing::info!("received routing command");
								routing_hub.handle_command(cmd);
							}
							Ok(None) => {} //Q: should we timeout? //TODO: check if this degrades perf at all
							Err(e) => panic!("Error reading routing command: {e:?}"),
						}
					}

					_ = tokio::signal::ctrl_c() => {
						info!("Service shutting down...");
						break;
					}
				}
			}

			Ok(())
		}
		Commands::Strategy { command } => {
			let redis_port = live_settings.config()?.redis_port;
			match command {
				StrategyCommands::Start => de_strategy::commands::start_listener(redis_port).await,
				StrategyCommands::Submit(args) => {
					let submit_args = de_strategy::commands::SubmitArgs {
						size_usdt: args.size_usdt,
						coin: args.coin,
						protocols: args.protocols,
						testnet: cli.testnet,
					};
					de_strategy::commands::submit(submit_args, redis_port).await
				}
			}
		}
		Commands::Risk { .. } | Commands::Routing { .. } | Commands::Init(_) => unreachable!(),
	});

	Ok(())
}

// TODO: change to initializing exchange sockets once, then just have a loop listening on localhost, that accepts new positions or modification requests.

#[instrument(skip(live_settings, tx, exchanges_arc))]
async fn command_new(position_args: PositionArgs, live_settings: Arc<LiveSettings>, tx: mpsc::Sender<PositionToHub>, exchanges_arc: Arc<Exchanges>) -> Result<()> {
	// Currently here mostly for purposes of checking server connectivity.
	let balance = exit_on_error(Exchanges::compile_total_balance(exchanges_arc.clone(), live_settings.clone()).await);
	log!("Current total available balance: {balance}");

	let (side, target_size) = match position_args.size_usdt {
		s if s > 0.0 => (Side::Buy, s),
		s if s < 0.0 => (Side::Sell, -s),
		_ => {
			bail!("Size must be non-zero");
		}
	};

	let followup_protocols = protocols::interpret_protocol_specs(position_args.followup_protocols).wrap_err("Failed to interpret followup protocols")?;
	let acquisition_protocols = protocols::interpret_protocol_specs(position_args.acquisition_protocols).wrap_err("Failed to interpret acquisition protocols")?;

	let spec = PositionSpec::new(position_args.coin, side, target_size);
	//let acquired = PositionAcquisition::dbg_new(spec).await?;
	let acquired = PositionAcquisition::do_acquisition(spec, acquisition_protocols, tx.clone(), exchanges_arc.clone()).await?;
	let _followed = PositionFollowup::do_followup(acquired, followup_protocols, tx.clone(), exchanges_arc.clone()).await?;

	Ok(())
}
