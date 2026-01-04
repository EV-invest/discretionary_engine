use v_utils::macros as v_macros;

fn __default_redis_port() -> u16 {
	6379
}

#[derive(Clone, Debug, Default, v_macros::MyConfigPrimitives, v_macros::SettingsNested)]
#[settings(use_env = true)]
pub struct StrategyConfig {
	#[serde(default = "__default_redis_port")]
	pub redis_port: u16,
}
