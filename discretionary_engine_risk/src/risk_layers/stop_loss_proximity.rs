use jiff::{Span, Unit};
use v_utils::{Percent, percent::PercentU};

use super::RiskLayerResult;

pub struct StopLossProximity {
	pub time: Span,
}

impl StopLossProximity {
	pub fn new(time: Span) -> Self {
		Self { time }
	}

	pub fn evaluate(&self) -> RiskLayerResult {
		let mul = self.mul_criterion();
		// mul ranges ~0.2-1.1. Tiers are separated by factor of e, so ln(mul) gives tier shift.
		// mul=1.0 -> ln=0 -> no change
		// mul=0.2 (fast SL) -> ln=-1.6 -> move ~1.6 tiers toward T (less risk)
		// mul=1.1 (slow SL) -> ln=+0.1 -> move ~0.1 tiers toward A (more risk)
		//TODO: non-linearity of pos/neg values (will tackle later)
		RiskLayerResult {
			adjustment: Percent(mul.ln()),
			certainty: PercentU::try_new(1.0).unwrap(),
		}
	}

	pub fn mul_criterion(&self) -> f64 {
		// 0.1 -> 0.2
		// 0.5 -> 0.5
		// 1 -> 0.7
		// 2 -> 0.8
		// 5 -> 0.9
		// 10+ -> ~1
		// Note: Using integer division to match old chrono::TimeDelta::num_hours() behavior
		let hours = (self.time.total(Unit::Second).unwrap() as i64 / 3600) as f64;

		// potentially transfer to just use something like `-1/(x+1) + 1` (to integrate would first need to fix snapshot, current one doesn't satisfy
		// methods for finding a better approximation: [../docs/assets/prof_advice_on_approximating_size_mul.pdf]

		(2.0 - (3.0_f64).powf(0.25) * (10.0_f64).powf(0.5) * hours.powf(0.25)).abs() / 10.0
	}
}

#[cfg(test)]
mod tests {
	use snapshot_fonts::SnapshotP;

	use super::*;

	#[test]
	fn proper_mul_snapshot_test() {
		//TODO!: switch to using non-homogeneous steps, so the data is dencer near 0 (requires: 1) new snapshot fn, 2) fn to gen it)
		let x_points: Vec<f64> = (0..1000).map(|x| (x as f64) / 10.0).collect();
		let mul_out: Vec<f64> = x_points
			.iter()
			.map(|x| StopLossProximity::new(Span::default().minutes((x * 60.0) as i64)).mul_criterion())
			.collect();
		let plot = SnapshotP::build(&mul_out).fallback(true /*HACK: wait for proper snapshot fonts impl*/).draw();

		insta::assert_snapshot!(plot, @r"
		                                                                       ▁▁▂▂▃▃▃▄▄▄▅▆▆▆▇▇▇██1.113
		                                                       ▁▁▂▂▃▃▄▄▅▅▆▆▇▇█████████████████████     
		                                           ▁▂▃▃▄▅▅▆▆▇▇████████████████████████████████████     
		                                ▁▂▂▃▅▅▆▇▇█████████████████████████████████████████████████     
		                        ▁▂▃▅▆▇▇███████████████████████████████████████████████████████████     
		                 ▁▃▄▅▆▇███████████████████████████████████████████████████████████████████     
		            ▂▃▅▆▇█████████████████████████████████████████████████████████████████████████     
		         ▄▆███████████████████████████████████████████████████████████████████████████████     
		      ▃▆██████████████████████████████████████████████████████████████████████████████████     
		    ▄█████████████████████████████████████████████████████████████████████████████████████     
		  ▂███████████████████████████████████████████████████████████████████████████████████████     
		▁▂████████████████████████████████████████████████████████████████████████████████████████0.200
		");
	}
}
