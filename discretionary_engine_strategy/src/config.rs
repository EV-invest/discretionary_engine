use serde::Deserialize;
use v_utils::macros as v_macros;

#[derive(Clone, Debug, Default, Deserialize, v_macros::SettingsNested)]
pub struct StrategyConfig {}
