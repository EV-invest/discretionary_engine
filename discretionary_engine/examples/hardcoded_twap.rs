//! Hardcoded TWAP executor with chase-limit per lot.
//!
//! Splits a total order into N equal lots over a fixed duration. Each lot gets one interval
//! slot. For 90% of that slot, we hold a PostOnly limit order that chases the best price
//! (amending whenever the market moves favorably — never retreating). At 90% we cancel and
//! market-fill whatever remains, guaranteeing completion within the interval. If the exchange
//! rejects the PostOnly immediately (spread too tight), we skip straight to market.
//!
//! Net effect: maker fills when the market cooperates, taker fills as a guaranteed fallback.
//! The interval sleep between lots starts *after* the lot completes, so total wall time is
//! `time + (lots - 1) * interval` in the worst case, or just `time` if all lots fill early.

use std::time::{Duration, Instant};

use color_eyre::eyre::{Context, ContextCompat, Result, bail};
use discretionary_engine::config::{LiveSettings, SettingsFlags};
use futures_util::{StreamExt, pin_mut};
use indicatif::{ProgressBar, ProgressStyle};
use nautilus_bybit::{
	common::enums::{BybitAccountType, BybitEnvironment, BybitOrderSide, BybitOrderType, BybitPositionSide, BybitProductType, BybitTimeInForce},
	http::{
		client::BybitRawHttpClient,
		models::{BybitInstrumentLinearResponse, BybitInstrumentSpotResponse},
		query::{BybitInstrumentsInfoParamsBuilder, BybitPositionListParamsBuilder, BybitTickersParamsBuilder, BybitWalletBalanceParams},
	},
	websocket::{
		client::BybitWebSocketClient,
		messages::{BybitWsAmendOrderParams, BybitWsCancelOrderParams, BybitWsPlaceOrderParams, NautilusWsMessage},
	},
};
use nautilus_model::identifiers::{ClientOrderId, InstrumentId, StrategyId, TraderId, VenueOrderId};
use secrecy::ExposeSecret;
use ustr::Ustr;
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
	/// Place one tick inside the spread instead of joining the best bid/ask queue.
	/// More likely to fill faster but exposes us to being exploited by spoofers.
	#[arg(long, default_value_t = false)]
	aggressive: bool,
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

	let client = BybitRawHttpClient::with_credentials(api_key.clone(), api_secret.clone(), None, None, None, None, None, None, None).context("Failed to create Bybit HTTP client")?;

	let is_spot = args.ticker.symbol.instrument == Instrument::Spot;
	let product_type = if is_spot { BybitProductType::Spot } else { BybitProductType::Linear };

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
					let size: f64 = coin_balance.wallet_balance.try_into().context("Failed to convert wallet balance to f64")?;
					if size == 0.0 {
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

	// Fetch qty_step and tick_size from instrument info
	let (qty_step, tick_size): (f64, f64) = {
		let info_params = BybitInstrumentsInfoParamsBuilder::default()
			.category(product_type)
			.symbol(symbol.clone())
			.build()
			.context("Failed to build instruments info params")?;
		if is_spot {
			let resp: BybitInstrumentSpotResponse = client
				.get_instruments::<BybitInstrumentSpotResponse>(&info_params)
				.await
				.context("Failed to fetch spot instrument info")?;
			let instr = resp.result.list.into_iter().next().with_context(|| format!("No spot instrument info for {symbol}"))?;
			(
				instr.lot_size_filter.base_precision.parse().context("Failed to parse base_precision")?,
				instr.price_filter.tick_size.parse().context("Failed to parse tick_size")?,
			)
		} else {
			let resp: BybitInstrumentLinearResponse = client
				.get_instruments::<BybitInstrumentLinearResponse>(&info_params)
				.await
				.context("Failed to fetch linear instrument info")?;
			let instr = resp.result.list.into_iter().next().with_context(|| format!("No linear instrument info for {symbol}"))?;
			(
				instr.lot_size_filter.qty_step.parse().context("Failed to parse qty_step")?,
				instr.price_filter.tick_size.parse().context("Failed to parse tick_size")?,
			)
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

	let bar = ProgressBar::new(args.lots as u64);
	bar.set_style(
		ProgressStyle::with_template("TWAP {msg} [{bar:40.cyan/blue}] {pos}/{len} lots  elapsed {elapsed_precise}  eta {eta_precise}")
			.unwrap()
			.progress_chars("█░"),
	);
	bar.set_message(symbol.clone());

	for i in 0..args.lots {
		let lot_num = i + 1;

		chase_lot(
			&client,
			api_key.clone(),
			api_secret.clone(),
			&symbol,
			product_type,
			order_side,
			category,
			size_per_lot,
			qty_step,
			tick_size,
			qty_decimals,
			interval,
			args.reduce_only,
			args.aggressive,
		)
		.await
		.with_context(|| format!("[{lot_num}/{}] chase_lot failed", args.lots))?;

		bar.inc(1);

		if lot_num < args.lots {
			tokio::time::sleep(interval).await;
		}
	}

	bar.finish_with_message(format!("{symbol} done"));
	println!("Done. Executed {total_size} {symbol} in {} lots.", args.lots);
	Ok(())
}

async fn chase_lot(
	http: &BybitRawHttpClient,
	api_key: String,
	api_secret: String,
	symbol: &str,
	product_type: BybitProductType,
	side: &str,
	category: &str,
	qty: f64,
	qty_step: f64,
	tick_size: f64,
	qty_decimals: usize,
	interval: Duration,
	reduce_only: bool,
	aggressive: bool,
) -> Result<()> {
	let deadline = Instant::now() + interval;
	let market_fallback_at = Instant::now() + interval.mul_f64(0.9);

	// Fetch initial bid/ask via HTTP
	let ticker_params = BybitTickersParamsBuilder::default()
		.category(product_type)
		.symbol(symbol.to_string())
		.build()
		.context("Failed to build ticker params")?;

	let (mut bid, mut ask): (f64, f64) = if product_type == BybitProductType::Spot {
		let resp: nautilus_bybit::http::models::BybitTickersSpotResponse = http
			.get_tickers::<nautilus_bybit::http::models::BybitTickersSpotResponse>(&ticker_params)
			.await
			.context("Failed to fetch spot ticker")?;
		let t = resp.result.list.into_iter().next().with_context(|| format!("No spot ticker for {symbol}"))?;
		(t.bid1_price.parse().context("bid1_price")?, t.ask1_price.parse().context("ask1_price")?)
	} else {
		let resp: nautilus_bybit::http::models::BybitTickersLinearResponse = http
			.get_tickers::<nautilus_bybit::http::models::BybitTickersLinearResponse>(&ticker_params)
			.await
			.context("Failed to fetch linear ticker")?;
		let t = resp.result.list.into_iter().next().with_context(|| format!("No linear ticker for {symbol}"))?;
		(t.bid1_price.parse().context("bid1_price")?, t.ask1_price.parse().context("ask1_price")?)
	};

	let bybit_side = match side {
		"Buy" => BybitOrderSide::Buy,
		"Sell" => BybitOrderSide::Sell,
		_ => bail!("Invalid side: {side}"),
	};

	let environment = BybitEnvironment::Mainnet;
	let trader_id = TraderId::from("TWAP-001");
	let strategy_id = StrategyId::from("TWAP-CHASE");
	let instrument_id = InstrumentId::from(format!("{symbol}.BYBIT").as_str());

	let mut trade_client = BybitWebSocketClient::new_trade(environment, Some(api_key), Some(api_secret), None, None);
	let mut market_client = BybitWebSocketClient::new_public_with(product_type, environment, None, None);

	trade_client.connect().await.context("Failed to connect trade WS")?;
	market_client.connect().await.context("Failed to connect market WS")?;
	trade_client.subscribe_orders().await.context("Failed to subscribe orders")?;
	market_client.subscribe_ticker(instrument_id).await.context("Failed to subscribe ticker")?;

	tokio::time::sleep(Duration::from_millis(500)).await;

	let trade_stream = trade_client.stream();
	let market_stream = market_client.stream();
	pin_mut!(trade_stream);
	pin_mut!(market_stream);

	let short_uuid = uuid::Uuid::new_v4().simple().to_string();
	let order_link_id = format!("twap-{short_uuid}");
	let client_order_id = ClientOrderId::from(order_link_id.as_str());

	let initial_price = limit_price(side, bid, ask, tick_size, aggressive);
	let initial_order = make_place_params(symbol, product_type, bybit_side, qty, qty_step, initial_price, tick_size, &order_link_id);
	trade_client
		.place_order(initial_order, client_order_id, trader_id, strategy_id, instrument_id)
		.await
		.context("Failed to place initial order")?;

	let mut last_price = initial_price;
	let mut venue_order_id: Option<VenueOrderId> = None;
	let mut filled = false;

	loop {
		if filled {
			break;
		}

		let now = Instant::now();

		if now >= deadline {
			break;
		}

		// At 90%, cancel limit and market-fill
		if now >= market_fallback_at {
			let cancel = BybitWsCancelOrderParams {
				category: product_type,
				symbol: Ustr::from(symbol),
				order_id: None,
				order_link_id: Some(order_link_id.clone()),
			};
			let _ = trade_client.cancel_order(cancel, client_order_id, trader_id, strategy_id, instrument_id, venue_order_id).await;
			// Market fallback via HTTP — simpler and avoids WS ordering issues
			let fallback_id = format!("twap-fb-{}", uuid::Uuid::new_v4());
			let resp = http
				.place_order(&serde_json::json!({
					"category": category,
					"symbol": symbol,
					"side": side,
					"orderType": "Market",
					"qty": format!("{qty:.qty_decimals$}"),
					"timeInForce": "IOC",
					"orderLinkId": fallback_id,
					"reduceOnly": reduce_only,
				}))
				.await
				.context("Market fallback order failed")?;
			if resp.ret_code != 0 {
				bail!("Market fallback failed: {} (code: {})", resp.ret_msg, resp.ret_code);
			}
			println!("  -> market fallback: id={:?}", resp.result.order_id);
			break;
		}

		let timeout = (market_fallback_at - now).min(Duration::from_millis(500));

		tokio::select! {
			Some(msg) = market_stream.next() => {
				if let NautilusWsMessage::Data(data_vec) = msg {
					for data in data_vec {
						if let nautilus_model::data::Data::Quote(quote) = data {
							bid = quote.bid_price.as_f64();
							ask = quote.ask_price.as_f64();
							let new_price = limit_price(side, bid, ask, tick_size, aggressive);
							let price_improved = match side {
								"Buy" => new_price > last_price,
								_ => new_price < last_price,
							};
							if price_improved {
								let amend = BybitWsAmendOrderParams {
									category: product_type,
									symbol: Ustr::from(symbol),
									order_id: None,
									order_link_id: Some(order_link_id.clone()),
									qty: Some(fmt_qty(qty, qty_step)),
									price: Some(fmt_price(new_price, tick_size)),
									trigger_price: None,
									take_profit: None,
									stop_loss: None,
									tp_trigger_by: None,
									sl_trigger_by: None,
								};
								if trade_client.amend_order(amend, client_order_id, trader_id, strategy_id, instrument_id, venue_order_id).await.is_ok() {
									last_price = new_price;
								}
							}
						}
					}
				}
			}
			Some(msg) = trade_stream.next() => {
				match msg {
					NautilusWsMessage::OrderStatusReports(reports) => {
						for report in reports {
							if report.client_order_id.as_ref().map(|c| c.to_string()) == Some(order_link_id.clone()) {
								venue_order_id = Some(report.venue_order_id);
								if report.filled_qty.as_f64() >= qty - qty_step * 0.5 {
									filled = true;
								}
							}
						}
					}
					NautilusWsMessage::FillReports(fills) => {
						for fill in &fills {
							if fill.client_order_id.as_ref().map(|c| c.to_string()) == Some(order_link_id.clone()) {
								println!("  -> fill: {} @ {}", fill.last_qty.as_f64(), fill.last_px.as_f64());
							}
						}
					}
					NautilusWsMessage::OrderRejected(rej) => {
						if rej.client_order_id.to_string() == order_link_id {
							// PostOnly rejected — go straight to market fallback
							let fallback_id = format!("twap-fb-{}", uuid::Uuid::new_v4());
							let resp = http.place_order(&serde_json::json!({
								"category": category,
								"symbol": symbol,
								"side": side,
								"orderType": "Market",
								"qty": format!("{qty:.qty_decimals$}"),
								"timeInForce": "IOC",
								"orderLinkId": fallback_id,
								"reduceOnly": reduce_only,
							})).await.context("Market fallback after rejection failed")?;
							if resp.ret_code != 0 {
								bail!("Market fallback failed: {} (code: {})", resp.ret_msg, resp.ret_code);
							}
							println!("  -> PostOnly rejected, market fallback: id={:?}", resp.result.order_id);
							filled = true;
						}
					}
					_ => {}
				}
			}
			_ = tokio::time::sleep(timeout) => {}
		}
	}

	trade_client.close().await.context("Failed to close trade WS")?;
	market_client.close().await.context("Failed to close market WS")?;

	Ok(())
}

fn limit_price(side: &str, bid: f64, ask: f64, tick: f64, aggressive: bool) -> f64 {
	match side {
		"Buy" =>
			if aggressive {
				let p = bid + tick;
				if p >= ask { bid } else { p }
			} else {
				bid
			},
		_ =>
			if aggressive {
				let p = ask - tick;
				if p <= bid { ask } else { p }
			} else {
				ask
			},
	}
}

fn make_place_params(symbol: &str, product_type: BybitProductType, side: BybitOrderSide, qty: f64, qty_step: f64, price: f64, tick: f64, order_link_id: &str) -> BybitWsPlaceOrderParams {
	BybitWsPlaceOrderParams {
		category: product_type,
		symbol: Ustr::from(symbol),
		side,
		order_type: BybitOrderType::Limit,
		qty: fmt_qty(qty, qty_step),
		market_unit: None,
		price: Some(fmt_price(price, tick)),
		time_in_force: Some(BybitTimeInForce::PostOnly),
		order_link_id: Some(order_link_id.to_string()),
		is_leverage: None,
		reduce_only: None,
		close_on_trigger: None,
		trigger_price: None,
		trigger_by: None,
		trigger_direction: None,
		tpsl_mode: None,
		take_profit: None,
		stop_loss: None,
		tp_trigger_by: None,
		sl_trigger_by: None,
		sl_trigger_price: None,
		tp_trigger_price: None,
		sl_order_type: None,
		tp_order_type: None,
		sl_limit_price: None,
		tp_limit_price: None,
	}
}

fn fmt_qty(qty: f64, step: f64) -> String {
	let decimals = step.to_string().find('.').map(|i| step.to_string().len() - i - 1).unwrap_or(0);
	format!("{qty:.decimals$}")
}

fn fmt_price(price: f64, tick: f64) -> String {
	let decimals = tick.to_string().find('.').map(|i| tick.to_string().len() - i - 1).unwrap_or(0);
	format!("{price:.decimals$}")
}
