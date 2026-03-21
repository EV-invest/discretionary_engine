use std::{collections::HashMap, path::PathBuf};

extern crate clap;

pub const EXE_NAME: &str = "discretionary_engine";

use color_eyre::eyre::{Result, eyre};
pub use de_core::config::ExchangeConfig;
pub use de_risk::config::*;
pub use de_strategy::config::*;
use v_exchanges::ExchangeName;
use v_utils::macros as v_macros;

#[derive(Clone, Debug, v_macros::LiveSettings, v_macros::MyConfigPrimitives, v_macros::Settings)]
#[settings(use_env = true)]
pub struct AppConfig {
	pub positions_dir: PathBuf,
	#[serde(default)]
	pub exchanges: HashMap<String, ExchangeConfig>,
	#[serde(default = "__default_comparison_offset_h")]
	pub comparison_offset_h: u32,
	#[serde(default = "__default_redis_port")]
	pub redis_port: u16,
	#[settings(flatten)]
	pub strategy: Option<StrategyConfig>,
	#[settings(flatten)]
	pub risk: Option<RiskConfig>,
}
impl AppConfig {
	pub fn get_exchange(&self, exchange: ExchangeName) -> Result<&ExchangeConfig> {
		self.exchanges.get(&exchange.to_string()).ok_or_else(|| eyre!("{exchange} exchange config not found"))
	}
}

fn __default_comparison_offset_h() -> u32 {
	24
}

fn __default_redis_port() -> u16 {
	6379
}
