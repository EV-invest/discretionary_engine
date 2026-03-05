//! Strategy command handlers.

use color_eyre::eyre::{Result, WrapErr};
use tracing::info;

use crate::{protocols::interpret_protocol_specs, redis_bus};

/// Arguments for submitting a position request.
#[derive(Clone, Debug)]
pub struct SubmitArgs {
	pub size_usdt: f64,
	pub coin: String,
	pub protocols: Vec<String>,
	pub testnet: bool,
}

/// Submit a position request via Redis.
pub async fn submit(args: SubmitArgs, redis_port: u16) -> Result<()> {
	// Validate protocols first
	let _protocols = interpret_protocol_specs(args.protocols.clone()).wrap_err("Invalid acquisition protocols")?;

	// Build CLI string and publish to Redis
	let cli_string = build_cli_string(&args);
	println!("Publishing command: {cli_string}");

	let mut conn = redis_bus::connect(redis_port).await?;
	let id = redis_bus::publish_command(&mut conn, &cli_string).await?;
	println!("Command published with ID: {id}");

	Ok(())
}
/// Start the strategy and listen for commands via Redis.
pub async fn start_listener(redis_port: u16) -> Result<()> {
	info!("Starting strategy, listening for commands on Redis port {redis_port}...");

	// Generate a unique consumer name
	let consumer_name = format!("strategy-{}", std::process::id());

	let mut conn = redis_bus::connect(redis_port).await?;
	let mut subscriber = redis_bus::subscribe_commands(&mut conn, &consumer_name).await?;

	info!("Listening for commands (Ctrl+C to exit)...");

	loop {
		tokio::select! {
			result = subscriber.next() => {
				match result {
					Ok(Some((id, cmd))) => {
						info!("Received command [{}]: {}", id, cmd);
						// TODO: Parse and forward to Nautilus Actor
						println!("[STRATEGY] Received: {}", cmd);
					}
					Ok(None) => {
						// Timeout, continue waiting
					}
					Err(e) => {
						tracing::error!("Error reading command: {}", e);
					}
				}
			}
			_ = tokio::signal::ctrl_c() => {
				info!("Shutting down...");
				break;
			}
		}
	}

	Ok(())
}
/// Reconstruct the CLI string from parsed args.
fn build_cli_string(args: &SubmitArgs) -> String {
	let mut parts = Vec::new();

	if args.testnet {
		parts.push("--testnet".to_string());
	}

	parts.push("submit".to_string());
	parts.push(format!("-s {}", args.size_usdt));
	parts.push(format!("-c {}", args.coin));

	for proto in &args.protocols {
		parts.push(format!("-f {proto}"));
	}

	parts.join(" ")
}
