use clap::{Args, Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr};
use discretionary_engine_strategy::{protocols::interpret_protocol_specs, redis_bus};
use futures_util as _;
use nautilus_bybit as _;
use nautilus_model as _;
use tracing::{info, level_filters::LevelFilter};

#[derive(Parser)]
#[command(author, version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"), about, long_about = None)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
	/// Use testnet instead of mainnet
	#[arg(long, global = true)]
	testnet: bool,
	/// Redis port for command communication
	#[arg(long, global = true, default_value = "6379", env = "REDIS_PORT")]
	redis_port: u16,
}

#[derive(Subcommand)]
enum Commands {
	/// Start the strategy and listen for commands via Redis
	Listen,
	/// Submit a new position request (sends to running strategy via Redis)
	Submit(SubmitArgs),
	/// Adjust an existing position (target qty or protocols)
	Adjust(AdjustArgs),
}

#[derive(Args, Clone, Debug)]
struct SubmitArgs {
	/// Target size of the position on the asset to establish. Signed.
	#[arg(short, long, allow_hyphen_values = true)]
	size_usdt: f64,
	/// _only_ the coin name itself. e.g. "BTC" or "ETH".
	/// It's engine's job to determine what pair and exchange to utilize
	//TODO!!!: allow providing a more precise primitive here (eg with Market, or with Market and Exchange); in which case it should understand that we want to skip engine suggestions for those, and for it to just accept the defined part of selection.
	#[arg(short, long)]
	asset: String,
	/// protocols parameters, in the format of "<protocol>-<params>", e.g. "ts:p0.5".
	/// Params consist of their starting letter followed by the value, e.g. "p0.5" for 0.5% offset. If multiple params are required, they are separated by '-'.
	/// For more info, reference [CompactFormat](v_utils::macros::CompactFormat)
	/// closing protocols
	#[arg(short, long)]
	closing: Vec<String>,
	/// opening protocols
	#[arg(short, long)]
	opening: Vec<String>,
}

#[derive(Args, Clone, Debug)]
struct AdjustArgs {
	sorry: (),
}

/// Reconstruct the CLI string from parsed args.
fn build_cli_string(args: &SubmitArgs, testnet: bool) -> String {
	let mut parts = Vec::new();

	if testnet {
		parts.push("--testnet".to_string());
	}

	parts.push("submit".to_string());
	parts.push(format!("-s {}", args.size_usdt));
	parts.push(format!("-c {}", args.asset));

	for p in &args.protocols {
		parts.push(format!("-a {p}")); //XXX: breaks everything
	}

	parts.join(" ")
}

#[tokio::main]
async fn main() -> Result<()> {
	v_utils::clientside!();

	let cli = Cli::parse();

	match cli.command {
		Commands::Listen => {
			info!("Starting strategy, listening for commands on Redis port {}...", cli.redis_port);

			// Generate a unique consumer name
			let consumer_name = format!("strategy-{}", std::process::id());

			let mut conn = redis_bus::connect(cli.redis_port).await?;
			let mut subscriber = redis_bus::subscribe_commands(&mut conn, &consumer_name).await?;

			info!("Listening for commands (Ctrl+C to exit)...");

			//LOOP: main loop
			loop {
				tokio::select! {
					result = subscriber.next() => {
						match result {
							Ok(Some((id, cmd))) => {
								info!("Received command [{id}]: {cmd}");
								// TODO: Parse and forward to Nautilus Actor
								println!("[STRATEGY] Received: {cmd}");
							}
							Ok(None) => {
								// Timeout, continue waiting
							}
							Err(e) => {
								tracing::error!("Error reading command: {e}");
							}
						}
					}
					_ = tokio::signal::ctrl_c() => {
						info!("Shutting down...");
						break;
					}
				}
			}
		}
		Commands::Submit(args) => {
			// Validate protocols first
			let _protocols = interpret_protocol_specs(args.protocols.clone()).wrap_err("Invalid protocols")?;

			// Build CLI string and publish to Redis
			let cli_string = build_cli_string(&args, cli.testnet);
			println!("Publishing command: {cli_string}");

			let mut conn = redis_bus::connect(cli.redis_port).await?;
			let id = redis_bus::publish_command(&mut conn, &cli_string).await?;
			println!("Command published with ID: {id}");
		}
		Commands::Adjust(args) => {
			//Q: think logic should be very similar right, - we just validate, then submit over into the actual execution. Just slightly different set of commands that could be passed here
			todo!();
		}
	}

	Ok(())
}
