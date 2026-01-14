use std::sync::Arc;

use color_eyre::eyre::{Context, Result, bail};
use nautilus_bybit::{
	common::enums::{BybitEnvironment, BybitProductType},
	http::query::{BybitInstrumentsInfoParamsBuilder, BybitTickersParamsBuilder},
};
use nautilus_model::identifiers::InstrumentId;
use secrecy::ExposeSecret;
use tracing::info;
use v_exchanges::Ticker;
use v_utils::{log, trades::Timeframe};

use crate::{bybit_common::*, config::LiveSettings};

#[derive(clap::Args, Debug)]
#[command(group(
    clap::ArgGroup::new("size_group")
        .required(true)
        .args(["quote", "notional", "size"]),
))]
pub(crate) struct AdjustPosArgs {
	/// Ticker to adjust position for.
	ticker: Ticker,

	/// Size in quote currency.
	#[arg(short = 'q', long)]
	quote: Option<f64>,

	/// Size in notional (USD)
	#[arg(short = 'n', long)]
	notional: Option<f64>,

	/// Size with suffix inference: "$" for USD, asset name (e.g., "BTC") for that asset, or plain number for quote
	#[arg(short = 's', long)]
	size: Option<String>,

	/// timeframe, in the format of "1m", "1h", "3M", etc.
	/// determines the target period for which we expect the edge to persist.
	#[arg(short, long)]
	tf: Option<Timeframe>,

	/// Reduce-only mode: only reduce existing position, don't increase it
	#[arg(long)]
	reduce: bool,

	/// Optional duration over which to execute the order (using chase-limit strategy)
	#[arg(short, long)]
	duration: Option<Timeframe>,
}

/// Round quantity to the appropriate step size
fn round_to_step(value: f64, step: f64) -> f64 {
	(value / step).round() * step
}

pub(crate) async fn main(args: AdjustPosArgs, live_settings: Arc<LiveSettings>, testnet: bool) -> Result<()> {
	info!("Starting adjust-pos for ticker: {:?}", args.ticker);

	// Determine whether we have a quote amount (quantity) or notional amount (USD)
	enum SizeType {
		Quote(f64),    // Actual quantity of asset
		Notional(f64), // USD value
		MinOrder(f64), // Multiple of minimum order size
	}

	let size_type = if let Some(notional) = args.notional {
		SizeType::Notional(notional)
	} else if let Some(quote) = args.quote {
		SizeType::Quote(quote)
	} else if let Some(size_str) = args.size {
		// Parse size with magnitude+unit framework:
		// - magnitude defaults to 1 if omitted
		// - unit defaults to quote if omitted
		// Examples: "min" -> 1min, "BTC" -> 1BTC, "5$" -> 5 USD, "10" -> 10 quote

		if size_str.ends_with('$') {
			// Strip $ and parse as USD
			let magnitude = size_str.trim_end_matches('$').parse::<f64>().context("Failed to parse USD amount from --size")?;
			SizeType::Notional(magnitude)
		} else if let Some(pos) = size_str.chars().position(|c| c.is_alphabetic()) {
			// Has a unit suffix like "BTC", "ETH", "min", etc.
			let (number_part, unit_part) = size_str.split_at(pos);

			// Parse magnitude, default to 1.0 if empty
			let magnitude = if number_part.is_empty() {
				1.0
			} else {
				number_part.parse::<f64>().context("Failed to parse magnitude from --size")?
			};

			// Handle different units
			if unit_part == "min" {
				SizeType::MinOrder(magnitude)
			} else {
				// TODO: Handle conversion from other assets to USD
				bail!("Asset conversion not yet implemented. Got {magnitude} {unit_part}, need to convert to USD");
			}
		} else {
			// Plain number - treat as quote currency (actual quantity)
			let qty = size_str.parse::<f64>().context("Failed to parse --size as number")?;
			SizeType::Quote(qty)
		}
	} else {
		bail!("No size specified");
	};

	// Create Bybit HTTP clients
	let exchange_name = args.ticker.exchange_name;
	let (raw_client, client) = create_bybit_clients(live_settings.clone(), exchange_name.clone(), testnet)?;

	// Get current ticker price first
	let symbol_raw = args.ticker.symbol.to_string();
	let symbol = convert_symbol_to_bybit(&symbol_raw);
	info!("Fetching current price for {symbol} (converted from {symbol_raw})");

	let ticker_params = BybitTickersParamsBuilder::default()
		.category(BybitProductType::Linear)
		.symbol(symbol.clone())
		.build()
		.context("Failed to build ticker params")?;

	let ticker_response: nautilus_bybit::http::models::BybitTickersLinearResponse = raw_client
		.get_tickers::<nautilus_bybit::http::models::BybitTickersLinearResponse>(&ticker_params)
		.await
		.context("Failed to fetch ticker data")?;

	let ticker = ticker_response.result.list.get(0).ok_or_else(|| color_eyre::eyre::eyre!("No ticker data found for {symbol}"))?;

	let current_price: f64 = ticker.last_price.parse().context("Failed to parse price as float")?;
	info!("Current price: ${current_price}");

	// Fetch instrument info to get lot size filter
	let instruments_params = BybitInstrumentsInfoParamsBuilder::default()
		.category(BybitProductType::Linear)
		.symbol(symbol.clone())
		.build()
		.context("Failed to build instruments info params")?;

	let instruments_response: nautilus_bybit::http::models::BybitInstrumentLinearResponse = raw_client
		.get_instruments::<nautilus_bybit::http::models::BybitInstrumentLinearResponse>(&instruments_params)
		.await
		.context("Failed to fetch instrument info")?;

	let instrument = instruments_response
		.result
		.list
		.get(0)
		.ok_or_else(|| color_eyre::eyre::eyre!("No instrument info found for {symbol}"))?;

	let lot_size_filter = &instrument.lot_size_filter;
	let qty_step: f64 = lot_size_filter.qty_step.parse().context("Failed to parse qtyStep")?;
	let min_order_qty: f64 = lot_size_filter.min_order_qty.parse().context("Failed to parse minOrderQty")?;
	let max_order_qty: f64 = lot_size_filter.max_order_qty.parse().context("Failed to parse maxOrderQty")?;

	let price_filter = &instrument.price_filter;
	let tick_size: f64 = price_filter.tick_size.parse().context("Failed to parse tickSize")?;

	info!("Instrument info - qtyStep: {qty_step}, tickSize: {tick_size}, minOrderQty: {min_order_qty}, maxOrderQty: {max_order_qty}");

	// Calculate quantity based on size type, extracting sign for order side
	let (raw_quantity, side) = match size_type {
		SizeType::Quote(qty) => (qty, if qty >= 0.0 { "Buy" } else { "Sell" }),
		SizeType::Notional(usd) => {
			let qty = usd / current_price;
			(qty, if usd >= 0.0 { "Buy" } else { "Sell" })
		}
		SizeType::MinOrder(multiplier) => {
			let qty = multiplier * min_order_qty;
			(qty, if multiplier >= 0.0 { "Buy" } else { "Sell" })
		}
	};

	// Work with absolute value for rounding
	let abs_raw_qty = raw_quantity.abs();
	let quantity = round_to_step(abs_raw_qty, qty_step);

	let actual_notional = quantity * current_price;
	log!("{} order: {:.6} -> rounded to {:.6} (notional: ${:.2})", side, abs_raw_qty, quantity, actual_notional);

	// Check if we should use chase-limit execution
	if args.duration.is_some() {
		log!("Using WebSocket chase-limit execution with duration: {:?}", args.duration);

		// Get API credentials for WebSocket
		let config = live_settings.config()?;
		let exchange_config = config.get_exchange(exchange_name)?;
		let api_key = exchange_config.api_pubkey.clone();
		let api_secret = exchange_config.api_secret.expose_secret().to_string();

		// Determine environment
		let environment = if testnet { BybitEnvironment::Testnet } else { BybitEnvironment::Mainnet };

		// Create InstrumentId for ticker subscription
		// Format: "SYMBOL.VENUE" e.g., "BTCUSDT.BYBIT"
		let instrument_id = InstrumentId::from(format!("{symbol}.BYBIT").as_str());

		let filled_qty = crate::ws_chase_limit::execute_ws_chase_limit(
			&raw_client, api_key, api_secret, environment, &symbol, instrument_id, side, quantity, qty_step, tick_size, args.duration,
		)
		.await
		.context("WebSocket chase-limit execution failed")?;

		let filled_notional = filled_qty * current_price;
		println!("✅ Chase-limit execution completed!");
		println!("   Filled: {:.6} {} (notional: ${:.2})", filled_qty, symbol, filled_notional);
		Ok(())
	} else {
		// Format quantity properly based on step size
		let qty_str = if qty_step >= 1.0 {
			format!("{:.0}", quantity)
		} else if qty_step >= 0.1 {
			format!("{:.1}", quantity)
		} else if qty_step >= 0.01 {
			format!("{:.2}", quantity)
		} else {
			format!("{quantity}")
		};

		// Prepare order request
		let order_request = serde_json::json!({
			"category": "linear",
			"symbol": symbol,
			"side": side,
			"orderType": "Market",
			"qty": qty_str,
			"timeInForce": "IOC",
			"orderLinkId": format!("adjust-{}", uuid::Uuid::new_v4()),
			"reduceOnly": args.reduce,
		});

		info!("Submitting market {side} order for {quantity} {symbol}");

		// Submit order
		let order_response = client.place_order(&order_request).await.context("Failed to place order")?;

		if order_response.ret_code == 0 {
			println!("✅ Order submitted successfully!");
			info!("Order submitted successfully!");
			if let Some(order_id) = order_response.result.order_id {
				println!("   Order ID: {order_id}");
			}
			println!("   Quantity: {} {} (notional: ${:.2})", qty_str, symbol, actual_notional);
			Ok(())
		} else {
			bail!("Order failed: {} (code: {})", order_response.ret_msg, order_response.ret_code);
		}
	}
}
