use std::{sync::Arc, time::Duration};

use clap::{Parser, Subcommand};
use de_risk::config::{LiveSettings, SettingsFlags};
use v_utils::utils::exit_on_error;

#[derive(Parser)]
#[command(author, version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"), about, long_about = None)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
	#[command(flatten)]
	settings: SettingsFlags,
}

#[derive(Subcommand)]
enum Commands {
	/// Calculate position size based on risk parameters
	Size,
	/// Show current balance across exchanges
	Balance,
}

#[tokio::main]
async fn main() {
	v_utils::clientside!();

	let cli = Cli::parse();
	let live_settings = Arc::new(exit_on_error(LiveSettings::new(cli.settings, Duration::from_secs(5))));

	match cli.command {
		Commands::Size => {
			unimplemented!("call through [main entrypoint](../../discretionary_engine/src/main.rs)")
		}
		Commands::Balance => {
			let config = exit_on_error(live_settings.config());
			let other_balances = config.risk.as_ref().and_then(|r| r.other_balances.as_ref());
			exit_on_error(de_risk::balance::main(&config.exchanges, other_balances).await);
		}
	}
}
