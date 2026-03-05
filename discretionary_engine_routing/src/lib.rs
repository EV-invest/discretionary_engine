pub mod algo;
use algo::ConceptualLimitArgs;
use miette::{Result, bail};

use crate::algo::ConceptualLimit;

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
		Commands::Deploy(args) => {
			let conceptual_limit = ConceptualLimit::from(args);

			dbg!(&conceptual_limit);

			bail!("should probably start receiving orders here");
		}
		Commands::Remove => bail!("todo"),
	}
}

//DO: want some kind of tracking system for all `ConceptualLimit`s in action
//DO: and then change/remove will naturally integrate with it
