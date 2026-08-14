use color_eyre::eyre::{Context, Result, bail};
use nautilus_bybit::{
	common::enums::BybitProductType,
	http::{
		client::{BybitHttpClient, BybitRawHttpClient},
		query::BybitTickersParamsBuilder,
	},
};
use tokio::time::{Duration, sleep};
use tracing::info;
use v_utils::{Timeframe, log};

/// Executes an order using a chase-limit strategy
///
/// This algorithm places a single limit order for the full quantity and continuously
/// cancels and replaces it at a better price as the market moves (one tick better than
/// the current best bid/ask, inside the spread if possible).
/// When duration expires, any unfilled quantity is executed with a market order.
///
/// # Arguments
/// * `raw_client` - Raw Bybit HTTP client for market data
/// * `client` - Authenticated Bybit HTTP client for order placement
/// * `symbol` - Trading symbol (Bybit format, e.g., "BTCUSDT")
/// * `side` - Order side ("Buy" or "Sell")
/// * `target_qty` - Total quantity to execute
/// * `price_tick` - Minimum price increment for the instrument
/// * `duration` - Optional duration to spread the execution over
pub async fn execute_chase_limit(
	raw_client: &BybitRawHttpClient,
	client: &BybitHttpClient,
	symbol: &str,
	side: &str,
	target_qty: f64,
	price_tick: f64,
	duration: Option<Timeframe>,
) -> Result<f64> {
	log!("Starting chase-limit execution for {side} {target_qty} {symbol}");

	// Calculate execution parameters based on duration
	let (sleep_interval, end_time) = if let Some(duration_tf) = duration {
		let total_duration_ms = duration_tf.0;
		let update_interval_ms = 1000; // Check/update every 1 second
		let end_time = std::time::Instant::now() + Duration::from_millis(total_duration_ms);
		log!("Patient execution over {duration_tf:?}: update_interval={update_interval_ms}ms");
		(Duration::from_millis(update_interval_ms), Some(end_time))
	} else {
		// Aggressive execution: update quickly
		log!("Aggressive execution: 500ms updates");
		(Duration::from_millis(500), None)
	};

	let base_order_link_id = format!("chase-{}", uuid::Uuid::new_v4());
	let mut current_order_link_id: Option<String> = None;
	let mut last_order_price: Option<f64> = None;
	let mut iteration = 0;

	//LOOP: very bad interface from the old implementation. Shouldn't be done this way, - will transfer to implementation through protocol primitives
	loop {
		iteration += 1;

		// Check if duration has expired
		if let Some(end) = end_time {
			if std::time::Instant::now() >= end {
				log!("Duration expired, placing final market order");

				// Cancel any existing limit order first using orderLinkId
				if let Some(ref order_link_id) = current_order_link_id {
					let cancel_request = serde_json::json!({
						"category": "linear",
						"symbol": symbol,
						"orderLinkId": order_link_id,
					});

					// Best effort cancel
					let _ = client.place_order(&cancel_request).await;
				}

				// Place market order for the full amount
				// The exchange will only fill what's remaining unfilled
				let market_request = serde_json::json!({
					"category": "linear",
					"symbol": symbol,
					"side": side,
					"orderType": "Market",
					"qty": format!("{}", target_qty),
					"timeInForce": "IOC",
					"orderLinkId": format!("{}-final", base_order_link_id),
				});

				let market_response = client.place_order(&market_request).await.context("Failed to place final market order")?;

				if market_response.ret_code == 0 {
					log!("Final market order placed successfully");
				} else {
					log!("Final market order result: {} (code: {})", market_response.ret_msg, market_response.ret_code);
				}

				break;
			}
		}

		// Get current best bid/ask
		let ticker_params = BybitTickersParamsBuilder::default()
			.category(BybitProductType::Linear)
			.symbol(symbol.to_string())
			.build()
			.context("Failed to build ticker params")?;

		let ticker_response: nautilus_bybit::http::models::BybitTickersLinearResponse = raw_client
			.get_tickers::<nautilus_bybit::http::models::BybitTickersLinearResponse>(&ticker_params)
			.await
			.context("Failed to fetch ticker data")?;

		let ticker = ticker_response.result.list.get(0).ok_or_else(|| color_eyre::eyre::eyre!("No ticker data found for {symbol}"))?;

		let bid_price: f64 = ticker.bid1_price.parse().context("Failed to parse bid price")?;
		let ask_price: f64 = ticker.ask1_price.parse().context("Failed to parse ask price")?;

		// Determine our limit price
		// - For buys: try to place one tick above current bid, but not crossing the spread
		// - For sells: try to place one tick below current ask, but not crossing the spread
		let limit_price = match side {
			"Buy" => {
				let improved_price = bid_price + price_tick;
				// Don't cross the spread
				if improved_price >= ask_price { bid_price } else { improved_price }
			}
			"Sell" => {
				let improved_price = ask_price - price_tick;
				// Don't cross the spread
				if improved_price <= bid_price { ask_price } else { improved_price }
			}
			_ => bail!("Invalid side: {side}"),
		};

		info!("[{iteration}] Market: bid={bid_price}, ask={ask_price}, target {side} limit @ {limit_price}");

		// Check if we should update the order
		// Only update if price changed significantly (to avoid unnecessary cancel-replace)
		let should_update = match last_order_price {
			Some(last_price) => (limit_price - last_price).abs() > price_tick * 0.5,
			None => true, // First order
		};

		if should_update {
			// Cancel existing order if there is one
			if let Some(ref old_order_link_id) = current_order_link_id {
				log!("[{iteration}] Cancelling previous order to update price");

				let cancel_request = serde_json::json!({
					"category": "linear",
					"symbol": symbol,
					"orderLinkId": old_order_link_id,
				});

				// Best effort cancel - don't fail if order is already filled
				let _ = client.place_order(&cancel_request).await;

				// Wait a moment for cancel to process
				sleep(Duration::from_millis(100)).await;
			}

			// Create new orderLinkId for this order
			let new_order_link_id = format!("{base_order_link_id}-{iteration}");

			// Place new order at updated price
			log!("[{iteration}] Placing {side} limit order: {target_qty} @ {limit_price}");

			let order_request = serde_json::json!({
				"category": "linear",
				"symbol": symbol,
				"side": side,
				"orderType": "Limit",
				"qty": format!("{}", target_qty),
				"price": format!("{}", limit_price),
				"timeInForce": "PostOnly",
				"orderLinkId": &new_order_link_id,
			});

			let order_response = client.place_order(&order_request).await.context("Failed to place chase-limit order")?;

			if order_response.ret_code == 0 {
				current_order_link_id = Some(new_order_link_id);
				last_order_price = Some(limit_price);
				info!("[{iteration}] Order placed successfully");
			} else if order_response.ret_code == 10001 || order_response.ret_msg.contains("post only") || order_response.ret_msg.contains("would cross") {
				info!("[{iteration}] PostOnly rejected (would cross spread): {}, will retry", order_response.ret_msg);
				// Don't update last_order_price or current_order_link_id, will retry next iteration
			} else {
				log!("Order placement warning: {} (code: {})", order_response.ret_msg, order_response.ret_code);
			}
		} else {
			info!("[{iteration}] Price unchanged, keeping order at {}", last_order_price.unwrap_or(0.0));
		}

		// Sleep before next iteration
		sleep(sleep_interval).await;

		// Safety check: don't run forever
		if iteration > 10000 {
			log!("Max iterations reached, stopping");
			break;
		}
	}

	log!("Chase-limit execution completed for {target_qty} {symbol}");
	Ok(target_qty)
}
