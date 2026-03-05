pub mod algo;
use algo::ConceptualLimitArgs;
use miette::{Result, bail};

#[derive(clap::Subcommand)]
pub enum Commands {
	Change,
	Deploy(ConceptualLimitArgs),
	Remove,
	//idk, still not sure if these would've been better
	//New,
	//Chg,
	//Del,
}

pub fn main(cmd: Commands) -> Result<()> {
	match cmd {
		Commands::Change => bail!("todo"),
		Commands::Deploy(args) => bail!("todo"),
		Commands::Remove => bail!("todo"),
	}
}
