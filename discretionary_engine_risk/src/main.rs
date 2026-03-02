use std::{sync::Arc, time::Duration};

use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;
use risk::config::{LiveSettings, SettingsFlags};

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
async fn main() -> Result<()> {
	v_utils::clientside!();

	let cli = Cli::parse();
	let live_settings = Arc::new(LiveSettings::new(cli.settings, Duration::from_secs(5))?);

	match cli.command {
		Commands::Size => {
			todo!();
		}
		Commands::Balance => {
			let config = live_settings.config()?;
			let other_balances = config.risk.as_ref().and_then(|r| r.other_balances);
			risk::balance::main(&config.exchanges, other_balances).await
		}
	}
}
