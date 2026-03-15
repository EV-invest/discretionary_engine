pub mod algo;
use algo::ConceptualLimitArgs;
use miette::{Result, bail};

use crate::algo::ConceptualLimit;

#[derive(clap::Subcommand)]
pub enum Commands {
	Adj,
	New(ConceptualLimitArgs),
	Del,
	//idk, still not sure if these would've been better
	//Change,
	//Deplay,
	//Remove,
}

pub fn main(cmd: Commands) -> Result<()> {
	match cmd {
		Commands::Adj => bail!("todo"),
		Commands::New(args) => {
			let conceptual_limit = ConceptualLimit::from(args);

			// we process all the errors and order rejections here
			// `ConceptualLimit` is basicall just a protocol, - it'll tell us what orders to have given current data. Only difference, - it returns exact orders not intent.

			dbg!(&conceptual_limit);

			bail!("should probably start receiving orders here");
		}
		Commands::Del => bail!("todo"),
	}
}

//DO: want some kind of tracking system for all `ConceptualLimit`s in action
//DO: and then change/remove will naturally integrate with it
//Nb: at this level there is no interpreting and selecting from orders generated from ConceptualLimit processes, - we just take and execute them as-is. Thinking about what others are doing is on `_strategy`, - in here we just do what we're told
