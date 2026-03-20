use serde::Deserialize;
use v_utils::macros as v_macros;

#[derive(Clone, Debug, Default, Deserialize, v_macros::SettingsNested)]
pub struct StrategyConfig {
	#[serde(default = "__default_redis_port")]
	pub redis_port: u16,
}
fn __default_redis_port() -> u16 {
	6379
}
