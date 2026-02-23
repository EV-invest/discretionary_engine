use v_utils::{Percent, percent::PercentU};

//TODO!!!: implement a metric to judge past perf through
use super::RiskLayerResult;

/// Risk layer that reduces size after a losing trade.
/// Procedurally reduces risk for psychological reasons - prevents revenge trading
/// and emotional decision-making after a loss.
/// Steps down 1 tier.
pub struct LostLastTrade;

impl LostLastTrade {
	pub fn evaluate(&self) -> RiskLayerResult {
		RiskLayerResult {
			adjustment: Percent(-1.0),
			certainty: PercentU::new(1.0).unwrap(),
		}
	}
}
