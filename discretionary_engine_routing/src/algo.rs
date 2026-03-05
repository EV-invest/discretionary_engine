#[derive(clap::Args, Clone, Debug)]
pub struct ConceptualLimitArgs {
	pub limit: f32,
	pub instrument: String,
}

#[derive(Debug, Clone, Default)]
pub struct ConceptualLimit {}
