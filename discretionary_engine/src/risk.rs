use std::sync::Arc;

use clap::{Args, Subcommand};
use color_eyre::eyre;
use de_risk::{
	FromPhone, LostLastTrade, Quality, RiskLayer, RiskTier, StopLossProximity, apply_risk_layers, apply_round_bias,
	balance::{RiskError, collect_balances, get_total_balance, initialize_exchanges},
	ema_prev_times_for_same_move,
};
use jiff::Unit;
use tracing::debug;
use v_exchanges::core::Ticker;
use v_utils::{Percent, log};

use crate::config::LiveSettings;

#[derive(Subcommand)]
pub enum RiskCommands {
	/// Calculate position size based on risk parameters
	Size(SizeArgs),
	/// Show current balance across exchanges
	Balance,
}

#[derive(Args, Debug, Default)]
pub struct SizeArgs {
	pub ticker: String,
	#[arg(short, long)]
	pub quality: Quality,
	#[arg(short, long, value_parser = parse_f64_with_underscores)]
	pub exact_sl: Option<f64>,
	#[arg(short, long)]
	pub percent_sl: Option<Percent>,
}
pub async fn size_main(live_settings: Arc<LiveSettings>, args: SizeArgs) -> Result<(), RiskError> {
	let config = live_settings.config().map_err(|e| eyre::eyre!(e))?;
	let risk_config = config.risk.as_ref().ok_or_else(|| color_eyre::eyre::eyre!("'risk' section missing from config"))?;
	let size_config = risk_config.size.as_ref().ok_or_else(|| color_eyre::eyre::eyre!("'risk.size' section missing from config"))?;
	let ticker: Ticker = args.ticker.parse()?;

	let exchanges = initialize_exchanges(&config.exchanges)?;
	let balances = collect_balances(&exchanges).await?;
	let total_balance = get_total_balance(&balances, risk_config.other_balances.as_ref());

	// Use the first exchange for price lookup
	let price = exchanges[0].price(ticker.symbol).await.unwrap();

	let sl_percent: Percent = match args.percent_sl {
		Some(percent) => percent,
		None => match args.exact_sl {
			Some(sl) => ((price - sl).abs() / price).into(),
			None => size_config.default_sl,
		},
	};
	let abs_max_risk = size_config.abs_max_risk;

	// Log risk tiers for reference
	let tier_strs: Vec<String> = RiskTier::non_test_tiers().map(|t| format!("{t:?}={}", t.risk_percent(abs_max_risk))).collect();
	log!("Risk tiers: {}, T=min", tier_strs.join(", "));

	let suggested_tier: RiskTier = args.quality.into();
	let base_risk = suggested_tier.risk_percent(abs_max_risk);
	log!("Suggested quality {:?} -> base risk {base_risk}", args.quality);

	// Build and apply risk layers based on config
	let risk_layers_config = size_config.risk_layers.as_ref();
	let use_sl_proximity = risk_layers_config.map(|c| c.stop_loss_proximity).unwrap_or(true);
	let use_from_phone = risk_layers_config.map(|c| c.from_phone).unwrap_or(false);
	let use_lost_last_trade = risk_layers_config.map(|c| c.lost_last_trade).unwrap_or(false);

	let mut risk_layers = Vec::default();
	if use_from_phone {
		log!("[RiskLayer::FromPhone] Enabled -> -1 tier");
		risk_layers.push(RiskLayer::FromPhone(FromPhone));
	}
	if use_lost_last_trade {
		log!("[RiskLayer::LostLastTrade] Enabled -> -1 tier");
		risk_layers.push(RiskLayer::LostLastTrade(LostLastTrade));
	}
	if use_sl_proximity {
		let time = ema_prev_times_for_same_move(exchanges[0].as_ref(), ticker.symbol, price, sl_percent).await?;
		let hours = (time.total(Unit::Second).unwrap() as i64 / 3600) as f64;
		let layer = StopLossProximity::new(time);
		log!("[RiskLayer::StopLossProximity] EMA time to SL: {hours:.1}h -> mul={:.3}", layer.mul_criterion());
		risk_layers.push(RiskLayer::StopLossProximity(layer));
	}

	let final_tier = apply_risk_layers(args.quality, risk_layers);
	let target_balance_risk = final_tier.risk_percent(abs_max_risk);

	// We bucket sizes into discrete tiers (poker-style) to simplify analysis and data collection,
	// accepting small EV loss vs perfect continuous sizing.
	let tier_shift = final_tier.as_index() as i32 - suggested_tier.as_index() as i32;
	if tier_shift != 0 {
		log!(
			"Risk adjustment: {} tier{} {} from {:?} -> {:?}",
			tier_shift.abs(),
			if tier_shift.abs() > 1 { "s" } else { "" },
			if tier_shift > 0 { "down" } else { "up" },
			args.quality,
			final_tier
		);
	}

	let is_test_quality = final_tier == RiskTier::T;
	debug!(?price, ?total_balance, ?final_tier);

	println!("Total Depo: {total_balance}$");
	println!("Chosen SL range: {sl_percent}");

	if is_test_quality {
		println!("Target Risk: N/A (test quality)");
		println!("\nSize: min");
	} else {
		let size = *total_balance * *(target_balance_risk / sl_percent);
		let biased_size = apply_round_bias(size, size_config.round_bias);
		println!("Target Risk: {target_balance_risk} of depo ({}) [tier {:?}]", total_balance * *target_balance_risk, final_tier);
		println!("\nSize: {biased_size:.2}");
	}
	Ok(())
}
pub async fn balance_main(live_settings: Arc<LiveSettings>) -> Result<(), RiskError> {
	let config = live_settings.config().map_err(|e| eyre::eyre!(e))?;
	let other_balances = config.risk.as_ref().and_then(|r| r.other_balances.as_ref());
	de_risk::balance::main(&config.exchanges, other_balances).await
}
fn parse_f64_with_underscores(s: &str) -> Result<f64, std::num::ParseFloatError> {
	s.replace('_', "").parse()
}
