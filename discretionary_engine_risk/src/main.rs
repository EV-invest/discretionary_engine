use clap::{Parser, Subcommand};
use color_eyre::eyre::Result;

#[derive(Parser)]
#[command(author, version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"), about, long_about = None)]
struct Cli {
	#[command(subcommand)]
	command: Commands,
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

	match cli.command {
		Commands::Size => {
			todo!();
		}
		Commands::Balance => {
			todo!();
		}
	}
}
