use std::ops::{Add, Sub};

use clap::ValueEnum;
use color_eyre::eyre::{Result, bail};
#[cfg(test)]
use insta as _;
use jiff::{Span, Timestamp, Unit};
use miette as _;
use strum::{EnumCount, EnumIter, IntoEnumIterator};
use tokio as _;
use tracing::debug;
use tracing_subscriber as _;
use v_exchanges::core::{Exchange, Symbol};
use v_utils::{Percent, percent::PercentU, trades::*};

pub mod balance;
pub mod config;
pub mod risk_layers;
pub use risk_layers::{FromPhone, LostLastTrade, RiskLayer, StopLossProximity, apply_risk_layers};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, ValueEnum)]
/// Quality of the trade setup
///
/// NB: can't choose based on what you feel like, - consult with exact criteria provided for each
/// NB: after a loss, we force it one step down. Compounds.
pub enum Quality {
	/// clear inefficiency AND entry within a clearly defined strategy AND strategy is historically profitable AND top tier situation within the strategy
	A,
	/// clear inefficiency AND entry within a clearly defined strategy AND strategy is historically profitable
	B,
	/// (entry within a clearly defined strategy AND strategy is historically profitable) OR (clear inefficiency)
	///
	/// NB: tends to be the most dangerous one. Already large enough to be felt, and much more common than the higher tiers; which leads to it ending up dominating %volume. And it's really tricky to trade this profitably consistently. In reality ends up blowing up risk while adding little (if anything) to flat EV.
	/// HINT: One of the telltale signs of most problematic types of this tier, is feeling that "it could evolve into something great". At higher tiers you usually get a feeling that formation is "complete" (even if scary), while large part of the reason for overtrading of `C` situaions, is hope and FOMO.
	C,
	/// looks good
	D,
	/// random test (uses exchange min size)
	T,
}

/// Risk tiers used for actual sizing. Rather deterministic rules for mapping to exact value from having this selected.
/// Only deterministic quantities like fees or expected slippage are applied from here on, to get the final size submitted to the execution engine.
///
/// Basically just [bucketing concept from poker](https://blog.gtowizard.com/the-magic-of-equity-buckets/)
#[derive(Clone, Copy, Debug, EnumCount, EnumIter, Eq, Ord, PartialEq, PartialOrd)]
pub enum RiskTier {
	A,
	B,
	C,
	D,
	E,
	F,
	G,
	H,
	T,
}

impl RiskTier {
	/// Returns all non-test tiers (A through H)
	pub fn non_test_tiers() -> impl Iterator<Item = RiskTier> {
		Self::iter().filter(|t| *t != RiskTier::T)
	}

	pub fn as_index(&self) -> usize {
		Self::iter().position(|t| t == *self).unwrap()
	}

	pub fn from_index(index: usize) -> Self {
		Self::iter().nth(index).unwrap_or(RiskTier::T)
	}

	pub fn risk_percent(&self, abs_max_risk: Percent) -> Percent {
		if *self == RiskTier::T {
			return Percent(0.0);
		}
		let e = std::f64::consts::E;
		Percent(*abs_max_risk / e.powi(self.as_index() as i32))
	}
}

impl Add<Percent> for RiskTier {
	type Output = RiskTier;

	/// Adjustment is in tier units (not percentage points).
	/// Positive adjustment moves toward A (lower index), negative toward T (higher index).
	fn add(self, rhs: Percent) -> Self::Output {
		let levels_to_move = (*rhs).round() as i32;
		let current_index = self.as_index() as i32;
		let max_index = (Self::COUNT - 1) as i32;
		#[allow(clippy::suspicious_arithmetic_impl)]
		let new_index = (current_index - levels_to_move).clamp(0, max_index) as usize;
		RiskTier::from_index(new_index)
	}
}

impl Sub<Percent> for RiskTier {
	type Output = RiskTier;

	fn sub(self, rhs: Percent) -> Self::Output {
		self + Percent(-*rhs)
	}
}

impl From<Quality> for RiskTier {
	fn from(quality: Quality) -> Self {
		match quality {
			Quality::A => RiskTier::A,
			Quality::B => RiskTier::B,
			Quality::C => RiskTier::C,
			Quality::D => RiskTier::D,
			Quality::T => RiskTier::T,
		}
	}
}

/// Returns EMA over previous 10 last moves of the same distance.
pub async fn ema_prev_times_for_same_move(exchange: &dyn Exchange, symbol: Symbol, price: f64, sl_percent: Percent) -> Result<Span> {
	static RUN_TIMES: usize = 10;
	let calc_range = |price: f64, sl_percent: Percent| {
		let sl = price * *sl_percent;
		(price - sl, price + sl)
	};
	let mut range = calc_range(price, sl_percent);
	let mut prev_time = Timestamp::now();
	let mut times: Vec<Span> = Vec::default();

	let mut check_if_satisfies = |k: &Kline, times: &mut Vec<Span>, prev_time: &mut Timestamp| -> bool {
		let new_anchor = match k {
			_ if k.low < range.0 => range.0,
			_ if k.high > range.1 => range.1,
			_ => return false,
		};
		let duration: Span = prev_time.since(k.open_time).unwrap();
		*prev_time = prev_time.checked_sub(duration).unwrap();
		times.push(duration);
		range = calc_range(new_anchor, sl_percent);
		true
	};

	let preset_timeframes: Vec<Timeframe> = vec!["1m".into(), "1h".into(), "1w".into()];
	let mut approx_correct_tf: Option<Timeframe> = None;
	for tf in preset_timeframes {
		if approx_correct_tf.is_none() {
			let klines = exchange.klines(symbol, tf, 1000.into()).await.unwrap();
			for k in klines.iter().rev() {
				match check_if_satisfies(k, &mut times, &mut prev_time) {
					true => {
						approx_correct_tf = Some(tf);
						break;
					}
					false => continue,
				}
			}
		}
	}

	let tf = approx_correct_tf.unwrap();
	let mut i = 0;
	while times.len() < RUN_TIMES && i < 10 {
		let request_range = (prev_time.checked_sub(tf.duration() * 999).unwrap(), prev_time);
		let klines = exchange.klines(symbol, tf, request_range.into()).await.unwrap();
		for k in klines.iter().rev() {
			match check_if_satisfies(k, &mut times, &mut prev_time) {
				true =>
					if times.len() == RUN_TIMES {
						break;
					},
				false => continue,
			}
		}
		i += 1;
	}

	if times.is_empty() {
		bail!("No data found for the given data & sl, you're on your own.");
	}
	debug!(?times);
	let ema = times
		.iter()
		.enumerate()
		.fold(0_i64, |acc: i64, (i, x): (usize, &Span)| acc + x.total(Unit::Second).unwrap() as i64 * (i as i64 + 1)) as f64
		/ ((times.len() + 1) as f64 * times.len() as f64 / 2.0);
	debug!(?ema);
	Ok(Span::new().seconds(ema as i64))
}

/// Apply rounding bias to skew the result towards rounder numbers.
/// The bias parameter (default 1%) determines how much to favor round numbers.
pub fn apply_round_bias(value: f64, bias: PercentU) -> f64 {
	if value == 0.0 {
		return value;
	}

	// Find the magnitude of the value (e.g., 1234.56 -> 1000)
	let magnitude = 10_f64.powi(value.abs().log10().floor() as i32);

	// Generate candidate round numbers at different scales
	let candidates = [
		// Round to nearest 1000, 500, 100, 50, 10, 5, 1
		(value / (magnitude * 10.0)).round() * (magnitude * 10.0),
		(value / (magnitude * 5.0)).round() * (magnitude * 5.0),
		(value / magnitude).round() * magnitude,
		(value / (magnitude / 2.0)).round() * (magnitude / 2.0),
		(value / (magnitude / 10.0)).round() * (magnitude / 10.0),
		(value / (magnitude / 20.0)).round() * (magnitude / 20.0),
		(value / (magnitude / 100.0)).round() * (magnitude / 100.0),
	];

	// Find the closest rounder number
	let closest_round = candidates
		.iter()
		.filter(|&&c| c > 0.0)
		.min_by(|&&a, &&b| {
			let dist_a = (a - value).abs();
			let dist_b = (b - value).abs();
			dist_a.partial_cmp(&dist_b).unwrap_or(std::cmp::Ordering::Equal)
		})
		.copied()
		.unwrap_or(value);

	// Apply bias: move towards the rounder number by the bias percentage
	value + (closest_round - value) * *bias
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn test_apply_round_bias() {
		// Test with 1% bias (default)
		let bias = PercentU::new(0.01).unwrap();

		// 1234.5 should move towards 1200 (closer round number)
		let result = apply_round_bias(1234.5, bias);
		assert!(result < 1234.5 && result > 1233.0, "Expected value between 1233 and 1234.5, got {result}");

		// 1250.0 is already round, should stay close
		let result = apply_round_bias(1250.0, bias);
		assert!((result - 1250.0).abs() < 1.0, "Expected value close to 1250, got {result}");

		// Test with higher bias (10%)
		let bias = PercentU::new(0.10).unwrap();
		let result = apply_round_bias(1234.5, bias);
		assert!(result < 1234.5 && result > 1230.0, "Expected larger shift with 10% bias, got {result}");

		// Test with zero value
		let result = apply_round_bias(0.0, bias);
		assert_eq!(result, 0.0, "Zero should remain zero");
	}
}
