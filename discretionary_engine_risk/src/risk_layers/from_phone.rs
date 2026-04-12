use v_utils::{Percent, percent::PercentU};

use super::RiskLayerResult;

/// Risk layer that reduces risk when trading from phone.
/// Steps down 1 tier.
pub struct FromPhone;

impl FromPhone {
	pub fn evaluate(&self) -> RiskLayerResult {
		RiskLayerResult {
			adjustment: Percent(-1.0),
			certainty: PercentU::try_new(1.0).unwrap(),
		}
	}
}
