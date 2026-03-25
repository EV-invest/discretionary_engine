use std::collections::HashMap;

use de_core::config::ExchangeConfig;
use v_utils::{Percent, macros as v_macros, percent::PercentU};

#[derive(Clone, Debug, v_macros::LiveSettings, v_macros::MyConfigPrimitives, v_macros::Settings)]
pub struct AppConfig {
	pub exchanges: HashMap<String, ExchangeConfig>,
	#[settings(flatten)]
	pub risk: Option<RiskConfig>,
}

#[derive(Clone, Debug, Default, v_macros::MyConfigPrimitives, v_macros::SettingsNested)]
pub struct RiskConfig {
	#[settings(flatten)]
	pub size: Option<SizeConfig>,
	pub other_balances: Option<HashMap<String, f64>>,
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
pub struct RiskLayersConfig {
	#[default(true)]
	pub stop_loss_proximity: bool,
	#[serde(default)]
	pub from_phone: bool,
	#[serde(default)]
	pub lost_last_trade: bool,
}
