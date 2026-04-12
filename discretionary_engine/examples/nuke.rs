use std::time::Duration;

use color_eyre::eyre::{Context, Result, bail};
use discretionary_engine::config::{LiveSettings, SettingsFlags};
use nautilus_bybit::{
	common::enums::{BybitPositionSide, BybitProductType},
	http::{client::BybitRawHttpClient, query::BybitPositionListParamsBuilder},
};
use secrecy::ExposeSecret;
use v_exchanges::Ticker;
use v_utils::trades::Timeframe;

#[derive(Debug, clap::Parser)]
struct Args {
	/// Ticker to close position for, e.g. "bybit:BTC-USDT.p"
	ticker: Ticker,
	/// Optional duration over which to close (chase-limit strategy) — not yet implemented in standalone
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

	let params = BybitPositionListParamsBuilder::default()
		.category(BybitProductType::Linear)
		.symbol(symbol.clone())
		.build()
		.context("Failed to build position params")?;

	let resp = client.get_positions(&params).await.context("Failed to fetch positions")?;

	if resp.result.list.is_empty() {
		println!("No position for {symbol}");
		return Ok(());
	}

	let position = &resp.result.list[0];
	let position_size: f64 = position.size.parse().context("Failed to parse position size")?;

	if position_size == 0.0 {
		println!("Position size is zero for {symbol}");
		return Ok(());
	}

	println!("Current position: {:?} {position_size} {symbol}", position.side);

	let order_side = if position.side == BybitPositionSide::Buy { "Sell" } else { "Buy" };

	let order_resp = client
		.place_order(&serde_json::json!({
			"category": "linear",
			"symbol": symbol,
			"side": order_side,
			"orderType": "Market",
			"qty": position_size.to_string(),
			"timeInForce": "IOC",
			"orderLinkId": format!("nuke-{}", uuid::Uuid::new_v4()),
			"reduceOnly": true,
		}))
		.await
		.context("Failed to place order")?;

	if order_resp.ret_code == 0 {
		println!("Position closed: {position_size} {symbol}");
		if let Some(id) = order_resp.result.order_id {
			println!("Order ID: {id}");
		}
		Ok(())
	} else {
		bail!("Order failed: {} (code: {})", order_resp.ret_msg, order_resp.ret_code);
	}
}
