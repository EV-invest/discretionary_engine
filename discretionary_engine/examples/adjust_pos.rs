use std::time::Duration;

use color_eyre::eyre::{Context, ContextCompat, Result, bail};
use discretionary_engine::config::{LiveSettings, SettingsFlags};
use nautilus_bybit::{
	common::enums::BybitProductType,
	http::{
		client::BybitRawHttpClient,
		query::{BybitInstrumentsInfoParamsBuilder, BybitTickersParamsBuilder},
	},
};
use secrecy::ExposeSecret;
use v_exchanges::Ticker;
use v_utils::trades::Timeframe;

#[derive(Debug, clap::Parser)]
#[command(group(
	clap::ArgGroup::new("size_group")
		.required(true)
		.args(["quote", "notional", "size"]),
))]
struct Args {
	/// Ticker to adjust position for, e.g. "bybit:BTC-USDT.p"
	ticker: Ticker,
	/// Size in quote currency (asset quantity)
	#[arg(short = 'q', long)]
	quote: Option<f64>,
	/// Size in notional (USD)
	#[arg(short = 'n', long)]
	notional: Option<f64>,
	/// Size with suffix: "$" for USD, "min" for minimum order size multiple, or plain number for quote
	#[arg(short = 's', long)]
	size: Option<String>,
	/// Reduce-only: only reduce existing position
	#[arg(long)]
	reduce: bool,
	/// Optional duration for chase-limit execution — not yet implemented in standalone
	#[arg(short, long)]
	duration: Option<Timeframe>,
	#[command(flatten)]
	settings: SettingsFlags,
}

#[tokio::main]
async fn main() -> Result<()> {
	color_eyre::install()?;
	use clap::Parser;
	let args = Args::parse();

	if args.duration.is_some() {
		unimplemented!("chase-limit execution not available in standalone example");
	}

	let live_settings = LiveSettings::new(args.settings, Duration::from_secs(5)).context("Failed to load config")?;
	let config = live_settings.config()?;
	let exchange_config = config.get_exchange(args.ticker.exchange_name.clone())?;
	let api_key = exchange_config.api_pubkey.clone();
	let api_secret = exchange_config.api_secret.expose_secret().to_string();

	let client = BybitRawHttpClient::with_credentials(api_key, api_secret, None, None, None, None, None, None, None).context("Failed to create Bybit HTTP client")?;

	let symbol = {
		let raw = args.ticker.symbol.to_string();
		raw.split('.').next().unwrap_or(&raw).replace('-', "").to_uppercase()
	};

	// Fetch current price
	let ticker_params = BybitTickersParamsBuilder::default()
		.category(BybitProductType::Linear)
		.symbol(symbol.clone())
		.build()
		.context("Failed to build ticker params")?;

	let ticker_resp: nautilus_bybit::http::models::BybitTickersLinearResponse = client
		.get_tickers::<nautilus_bybit::http::models::BybitTickersLinearResponse>(&ticker_params)
		.await
		.context("Failed to fetch ticker")?;

	let current_price: f64 = ticker_resp
		.result
		.list
		.first()
		.with_context(|| format!("No ticker data for {symbol}"))?
		.last_price
		.parse()
		.context("Failed to parse price")?;

	// Fetch instrument info for lot size
	let instruments_params = BybitInstrumentsInfoParamsBuilder::default()
		.category(BybitProductType::Linear)
		.symbol(symbol.clone())
		.build()
		.context("Failed to build instruments info params")?;

	let instruments_resp: nautilus_bybit::http::models::BybitInstrumentLinearResponse = client
		.get_instruments::<nautilus_bybit::http::models::BybitInstrumentLinearResponse>(&instruments_params)
		.await
		.context("Failed to fetch instrument info")?;

	let instrument = instruments_resp.result.list.first().with_context(|| format!("No instrument info for {symbol}"))?;
	let qty_step: f64 = instrument.lot_size_filter.qty_step.parse().context("Failed to parse qtyStep")?;
	let min_order_qty: f64 = instrument.lot_size_filter.min_order_qty.parse().context("Failed to parse minOrderQty")?;

	// Resolve size to (raw_quantity, side)
	let (raw_quantity, side) = if let Some(notional) = args.notional {
		let qty = notional / current_price;
		(qty, if notional >= 0.0 { "Buy" } else { "Sell" })
	} else if let Some(quote) = args.quote {
		(quote, if quote >= 0.0 { "Buy" } else { "Sell" })
	} else if let Some(size_str) = args.size {
		if size_str.ends_with('$') {
			let usd: f64 = size_str.trim_end_matches('$').parse().context("Failed to parse USD amount")?;
			let qty = usd / current_price;
			(qty, if usd >= 0.0 { "Buy" } else { "Sell" })
		} else if let Some(pos) = size_str.chars().position(|c| c.is_alphabetic()) {
			let (number_part, unit_part) = size_str.split_at(pos);
			let magnitude = if number_part.is_empty() {
				1.0
			} else {
				number_part.parse::<f64>().context("Failed to parse magnitude")?
			};
			if unit_part == "min" {
				let qty = magnitude * min_order_qty;
				(qty, if magnitude >= 0.0 { "Buy" } else { "Sell" })
			} else {
				bail!("Asset conversion not yet implemented. Got {magnitude} {unit_part}");
			}
		} else {
			let qty: f64 = size_str.parse().context("Failed to parse size as number")?;
			(qty, if qty >= 0.0 { "Buy" } else { "Sell" })
		}
	} else {
		bail!("No size specified");
	};

	let abs_qty = raw_quantity.abs();
	let quantity = (abs_qty / qty_step).round() * qty_step;
	let actual_notional = quantity * current_price;

	println!("{side} {abs_qty:.6} -> rounded {quantity:.6} {symbol} (notional: ${actual_notional:.2})");

	let qty_str = format!("{quantity}");

	let order_resp = client
		.place_order(&serde_json::json!({
			"category": "linear",
			"symbol": symbol,
			"side": side,
			"orderType": "Market",
			"qty": qty_str,
			"timeInForce": "IOC",
			"orderLinkId": format!("adjust-{}", uuid::Uuid::new_v4()),
			"reduceOnly": args.reduce,
		}))
		.await
		.context("Failed to place order")?;

	if order_resp.ret_code == 0 {
		println!("Order placed: {qty_str} {symbol}");
		if let Some(id) = order_resp.result.order_id {
			println!("Order ID: {id}");
		}
		Ok(())
	} else {
		bail!("Order failed: {} (code: {})", order_resp.ret_msg, order_resp.ret_code);
	}
}
