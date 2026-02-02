use std::{collections::HashMap, path::PathBuf};

extern crate clap;

pub const EXE_NAME: &str = "discretionary_engine";

use color_eyre::eyre::{Result, eyre};
pub use discretionary_engine_strategy::config::*;
use secrecy::SecretString;
use v_exchanges::ExchangeName;
use v_utils::{Percent, macros as v_macros, percent::PercentU};

#[derive(Clone, Debug, v_macros::LiveSettings, v_macros::MyConfigPrimitives, v_macros::Settings)]
#[settings(use_env = true)]
pub struct AppConfig {
	pub positions_dir: PathBuf,
	#[serde(default)]
	pub exchanges: HashMap<String, ExchangeConfig>,
	#[serde(default = "__default_comparison_offset_h")]
	pub comparison_offset_h: u32,
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

#[derive(Clone, Debug, v_macros::MyConfigPrimitives)]
pub struct ExchangeConfig {
	pub api_pubkey: String,
	pub api_secret: SecretString,
	#[serde(default)]
	pub passphrase: Option<SecretString>,
}
#[derive(Clone, Debug, Default, v_macros::MyConfigPrimitives, v_macros::SettingsNested)]
pub struct RiskConfig {
	#[settings(flatten)]
	pub size: Option<SizeConfig>,
	pub other_balances: Option<f64>,
}
#[derive(Clone, Debug, Default, v_macros::MyConfigPrimitives, v_macros::SettingsNested)]
pub struct SizeConfig {
	pub default_sl: Percent,
	#[settings(default = "PercentU::new(0.01).unwrap()")]
	pub round_bias: PercentU,
	/// Max risk for A-quality trades. Each tier below divides by e (2.718...)
	pub abs_max_risk: Percent,
	#[settings(flatten)]
	pub risk_layers: Option<RiskLayersConfig>,
}
#[derive(Clone, Debug, v_macros::MyConfigPrimitives, v_macros::SettingsNested, smart_default::SmartDefault)]
#[serde(default)]
pub struct RiskLayersConfig {
	#[default(true)]
	#[serde(default)]
	pub stop_loss_proximity: bool,
	#[serde(default)]
	pub from_phone: bool,
	#[serde(default)]
	pub lost_last_trade: bool,
}
fn __default_comparison_offset_h() -> u32 {
	24
}
