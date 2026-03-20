pub mod algo;
pub mod data;

use std::collections::HashSet;

use de_core::component::{Component, ComponentState, ComponentTrigger};

use crate::algo::ConceptualLimit;

#[derive(clap::Subcommand)]
pub enum Commands {
	Adj,
	New(algo::ConceptualLimitArgs),
	Del,
	//idk, still not sure if these would've been better
	//Change,
	//Deplay,
	//Remove,
}

#[derive(Debug, derive_more::Display, derive_more::From)]
pub enum RoutingError {
	Invalid(InvalidRoutingError),
	Other(miette::Error),
}

#[derive(Debug, derive_more::Display)]
pub enum InvalidRoutingError {
	#[display("adjustment would reverse position")]
	AdjustmentWouldReverse,
}

pub fn main(cmd: Commands) -> Result<(), RoutingError> {
	match cmd {
		Commands::New(args) => {
			let conceptual_limit = ConceptualLimit::from(args);

			// we process all the errors and order rejections here
			// `ConceptualLimit` is basically just a protocol, - it'll tell us what orders to have given current data. Only difference, - it returns exact orders not intent.

			dbg!(&conceptual_limit);

			Ok(())
		}
		//DO: my current understanding is Adj and Del can attempt to directly change the outstanding ConceptualLimit protocols. Since they are exclusively pulled, and also at this level we do not generate new opinions on delta intent, we can just instantly accept whatever suggestion inputted
		//DO: thus these will just directly change values on list of ConceptualLimits we're listening on atm
		Commands::Adj => Err(RoutingError::Other(miette::miette!("todo"))),
		Commands::Del => Err(RoutingError::Other(miette::miette!("todo"))),
	}
}

#[derive(Debug, derive_more::Deref)]
struct RoutingHub {
	#[deref]
	conceptual_limits: HashSet<ConceptualLimit>,
	state: ComponentState,
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
