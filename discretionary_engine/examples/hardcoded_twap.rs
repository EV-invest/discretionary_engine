use color_eyre::eyre::{Context, ContextCompat, Result, bail};
use nautilus_bybit::{
	common::enums::{BybitPositionSide, BybitProductType},
	http::{client::BybitRawHttpClient, query::BybitPositionListParamsBuilder},
};
use v_exchanges::Ticker;
use v_utils::{
	io::{ConfirmResult, confirmation},
	trades::{Side, Timeframe},
};


#[derive(clap::Parser, Debug)]
struct Args {
	/// Ticker, e.g. "bybit:BTC-USDT.p"
	ticker: Ticker,
	#[arg(long)]
	side: Side,
	/// Positive f64 or "all" (sell-only: uses current position size)
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
}

#[tokio::main]
async fn main() -> Result<()> {
	color_eyre::install()?;
	use clap::Parser;
	let args = Args::parse();

	let api_key = std::env::var("BYBIT_TIGER_FULL_KEY").context("BYBIT_TIGER_FULL_KEY not set")?;
	let api_secret = std::env::var("BYBIT_TIGER_FULL_SECRET").context("BYBIT_TIGER_FULL_SECRET not set")?;

	let client = BybitRawHttpClient::with_credentials(api_key, api_secret, None, None, None, None, None, None, None)
		.context("Failed to create Bybit HTTP client")?;

	// "BTC-USDT.p" -> "BTCUSDT"
	let symbol = {
		let raw = args.ticker.symbol.to_string();
		raw.split('.').next().unwrap_or(&raw).replace('-', "").to_uppercase()
	};

	let total_size: f64 = match args.quantity.trim() {
		"all" => match args.side {
			Side::Sell => {
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
			}
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

	let interval = args.time.duration() / args.lots as u32;
	let size_per_lot = total_size / args.lots as f64;

	let summary = format!(
		"TWAP: {} {} {total_size} {symbol} | {} lots of {size_per_lot:.6} every {}s (total {}s)",
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

	for i in 0..args.lots {
		let lot_num = i + 1;
		let resp = client
			.place_order(&serde_json::json!({
				"category": "linear",
				"symbol": symbol,
				"side": order_side,
				"orderType": "Market",
				"qty": format!("{size_per_lot:.6}"),
				"timeInForce": "IOC",
				"orderLinkId": format!("twap-{}-{}", lot_num, uuid::Uuid::new_v4()),
			"reduceOnly": args.reduce_only,
			}))
			.await
			.context("Failed to place order")?;

		if resp.ret_code != 0 {
			bail!("[{lot_num}/{}] Order failed: {} (code: {})", args.lots, resp.ret_msg, resp.ret_code);
		}
		println!("[{lot_num}/{}] {order_side} {size_per_lot:.6} {symbol} — id: {:?}", args.lots, resp.result.order_id);

		if lot_num < args.lots {
			tokio::time::sleep(interval).await;
		}
	}

	println!("Done. Executed {total_size} {symbol} in {} lots.", args.lots);
	Ok(())
}
