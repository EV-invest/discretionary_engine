use std::time::Duration;

use color_eyre::eyre::{Context, ContextCompat, Result, bail};
use discretionary_engine::config::{LiveSettings, SettingsFlags};
use nautilus_bybit::{
	common::enums::{BybitAccountType, BybitPositionSide, BybitProductType},
	http::{
		client::BybitRawHttpClient,
		models::{BybitInstrumentLinearResponse, BybitInstrumentSpotResponse},
		query::{BybitInstrumentsInfoParamsBuilder, BybitPositionListParamsBuilder, BybitWalletBalanceParams},
	},
};
use secrecy::ExposeSecret;
use v_exchanges::{Instrument, Ticker};
use v_utils::{
	io::{ConfirmResult, confirmation},
	trades::{Side, Timeframe},
};

#[derive(Debug, clap::Parser)]
struct Args {
	/// Ticker, e.g. "bybit:BTC-USDT.p" (perp) or "bybit:TWT-USDT" (spot)
	ticker: Ticker,
	#[arg(long)]
	side: Side,
	/// Positive f64 or "all" (sell-only: uses current position/balance size)
	#[arg(short = 'q', long)]
	quantity: String,
	/// Total duration, e.g. "1h", "30m"
	#[arg(short = 't', long)]
	time: Timeframe,
	/// Number of lots
	#[arg(short = 'l', long)]
	lots: u8,
	#[arg(long)]
	reduce_only: bool,
	#[command(flatten)]
	settings: SettingsFlags,
}

#[tokio::main]
async fn main() -> Result<()> {
	color_eyre::install()?;
	use clap::Parser;
	let args = Args::parse();

	let live_settings = LiveSettings::new(args.settings, Duration::from_secs(5)).context("Failed to load config")?;
	let config = live_settings.config()?;
	let exchange_config = config.get_exchange(args.ticker.exchange_name.clone())?;
	let api_key = exchange_config.api_pubkey.clone();
	let api_secret = exchange_config.api_secret.expose_secret().to_string();

	let client = BybitRawHttpClient::with_credentials(api_key, api_secret, None, None, None, None, None, None, None).context("Failed to create Bybit HTTP client")?;

	let is_spot = args.ticker.symbol.instrument == Instrument::Spot;

	let symbol = {
		let raw = args.ticker.symbol.to_string();
		raw.split('.').next().unwrap_or(&raw).replace('-', "").to_uppercase()
	};

	let base_coin = args.ticker.symbol.pair.base().to_string().to_uppercase();

	let total_size: f64 = match args.quantity.trim() {
		"all" => match args.side {
			Side::Sell =>
				if is_spot {
					let params = BybitWalletBalanceParams {
						account_type: BybitAccountType::Unified,
						coin: Some(base_coin.clone()),
					};
					let resp = client.get_wallet_balance(&params).await.context("Failed to fetch wallet balance")?;
					let coin_balance = resp
						.result
						.list
						.iter()
						.flat_map(|w| w.coin.iter())
						.find(|c| c.coin.as_str().eq_ignore_ascii_case(&base_coin))
						.with_context(|| format!("No {base_coin} balance found in wallet"))?;
					let size = coin_balance.wallet_balance.try_into().context("Failed to convert wallet balance to f64")?;
					if size == 0.0_f64 {
						bail!("Wallet balance is zero for {base_coin}");
					}
					size
				} else {
					let params = BybitPositionListParamsBuilder::default()
						.category(BybitProductType::Linear)
						.symbol(symbol.clone())
						.build()
						.context("Failed to build position params")?;
					let resp = client.get_positions(&params).await.context("Failed to fetch positions")?;
					let pos = resp.result.list.first().with_context(|| format!("No open position for {symbol}"))?;
					let size: f64 = pos.size.parse().context("Failed to parse position size")?;
					if size == 0.0 {
						bail!("Position size is zero for {symbol}");
					}
					if pos.side != BybitPositionSide::Buy {
						bail!("Expected long position to sell, got {:?}", pos.side);
					}
					size
				},
			Side::Buy => unimplemented!("quantity=all with side=buy"),
		},
		q => {
			let v: f64 = q.parse().with_context(|| format!("Invalid quantity '{q}'"))?;
			if v <= 0.0 {
				bail!("quantity must be positive");
			}
			v
		}
	};

	// Fetch qty_step (or base_precision for spot) to round lot sizes correctly
	let qty_step: f64 = {
		let info_params = BybitInstrumentsInfoParamsBuilder::default()
			.category(if is_spot { BybitProductType::Spot } else { BybitProductType::Linear })
			.symbol(symbol.clone())
			.build()
			.context("Failed to build instruments info params")?;
		if is_spot {
			let resp: BybitInstrumentSpotResponse = client
				.get_instruments::<BybitInstrumentSpotResponse>(&info_params)
				.await
				.context("Failed to fetch spot instrument info")?;
			let instr = resp.result.list.into_iter().next().with_context(|| format!("No spot instrument info for {symbol}"))?;
			instr.lot_size_filter.base_precision.parse().context("Failed to parse base_precision")?
		} else {
			let resp: BybitInstrumentLinearResponse = client
				.get_instruments::<BybitInstrumentLinearResponse>(&info_params)
				.await
				.context("Failed to fetch linear instrument info")?;
			let instr = resp.result.list.into_iter().next().with_context(|| format!("No linear instrument info for {symbol}"))?;
			instr.lot_size_filter.qty_step.parse().context("Failed to parse qty_step")?
		}
	};

	let qty_decimals = qty_step.to_string().find('.').map(|i| qty_step.to_string().len() - i - 1).unwrap_or(0);

	let interval = args.time.duration() / args.lots as u32;
	let size_per_lot = (total_size / args.lots as f64 / qty_step).floor() * qty_step;

	let summary = format!(
		"TWAP: {} {} {total_size} {symbol} | {} lots of {size_per_lot:.qty_decimals$} every {}s (total {}s)",
		args.ticker,
		args.side,
		args.lots,
		interval.as_secs(),
		args.time.duration().as_secs(),
	);

	if confirmation(&summary).flush_blocking() != ConfirmResult::Yes {
		bail!("Aborted.");
	}

	let order_side = match args.side {
		Side::Buy => "Buy",
		Side::Sell => "Sell",
	};
	let category = if is_spot { "spot" } else { "linear" };

	for i in 0..args.lots {
		let lot_num = i + 1;
		let resp = client
			.place_order(&serde_json::json!({
				"category": category,
				"symbol": symbol,
				"side": order_side,
				"orderType": "Market",
				"qty": format!("{size_per_lot:.qty_decimals$}"),
				"timeInForce": "IOC",
				"orderLinkId": format!("twap-{}-{}", lot_num, uuid::Uuid::new_v4()),
				"reduceOnly": args.reduce_only,
			}))
			.await
			.context("Failed to place order")?;

		if resp.ret_code != 0 {
			bail!("[{lot_num}/{}] Order failed: {} (code: {})", args.lots, resp.ret_msg, resp.ret_code);
		}
		println!("[{lot_num}/{}] {order_side} {size_per_lot:.qty_decimals$} {symbol} — id: {:?}", args.lots, resp.result.order_id);

		if lot_num < args.lots {
			tokio::time::sleep(interval).await;
		}
	}

	println!("Done. Executed {total_size} {symbol} in {} lots.", args.lots);
	Ok(())
}
