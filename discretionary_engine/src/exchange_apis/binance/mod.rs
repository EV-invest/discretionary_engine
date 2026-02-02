#![allow(non_snake_case, dead_code)]
use tracing::{info, trace};
pub mod info;
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct BinanceExchange {
	pub binance_futures_info: BinanceExchangeFutures,
}
impl BinanceExchange {
	#[instrument(skip_all)]
	pub async fn init(live_settings: Arc<LiveSettings>) -> Result<Self> {
		let binance_futures_info = BinanceExchangeFutures::init(live_settings.clone()).await?;
		Ok(Self { binance_futures_info })
	}

	// Finds all pairs with the given base asset, returns absolute minimal order trade size for it.
	//TODO!!: switch to requesting full orders. This is not general, so for limit and stop market orders must know the offset to determine the accurate min_qty.
	#[instrument(skip(self))]
	pub fn min_qties_batch(&self, base_asset: &str, ordertypes: &[ConceptualOrderType]) -> Vec<f64> {
		assert_ne!(*self, Self::default());

		let mut min_qties = Vec::new();
		for s in &self.binance_futures_info.symbols {
			if s.base_asset == *base_asset {
				let mut all_min_notionals_for_asset = Vec::new();
				for ordertype in ordertypes {
					all_min_notionals_for_asset.push(s.min_trade_qty_notional(ordertype));
				}
				//- other sub-markets
				assert!(!all_min_notionals_for_asset.is_empty(), "No such asset found in the exchange info");
				min_qties.push(all_min_notionals_for_asset.iter().sum());
			}
		}

		min_qties
	}

	#[instrument(skip(self))]
	pub fn min_qty_any_ordertype(&self, base_asset: &str) -> f64 {
		let mut on_different_pairs = Vec::new();
		for s in &self.binance_futures_info.symbols {
			if s.base_asset == *base_asset {
				//HACK: just assumes that there is no way to hit a smaller min_qty limit by placing a limit order, no matter at what offset to the price.
				on_different_pairs.push(s.min_trade_qty_notional(&ConceptualOrderType::Market(ConceptualMarket::new(Percent(1.0)))));
			}
		}
		on_different_pairs.iter().sum()
	}

	#[instrument(skip(self))]
	pub fn pair(&self, base_asset: &str, quote_asset: &str) -> Option<&info::FuturesSymbol> {
		//? Should I cast `to_uppercase()`?
		self.binance_futures_info.symbols.iter().find(|s| s.base_asset == base_asset && s.quote_asset == quote_asset)
	}
}

#[instrument(skip(key, secret))]
pub async fn signed_request<S: AsRef<str>>(http_method: reqwest::Method, endpoint_str: &str, mut params: HashMap<&'static str, String>, key: S, secret: S) -> Result<reqwest::Response> {
	let mut headers = HeaderMap::new();
	headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json;charset=utf-8"));
	headers.insert("X-MBX-APIKEY", HeaderValue::from_str(key.as_ref())?);
	let client = reqwest::Client::builder().default_headers(headers).build()?;

	let max_retries = 10;
	let mut retry_delay = std::time::Duration::from_secs(1);
	let mut encountered_cloudfront_error = false;

	for attempt in 0..max_retries {
		let time_ms = Utc::now().timestamp_millis();
		params.insert("timestamp", format!("{time_ms}"));

		let query_string = serde_urlencoded::to_string(&params)?;

		let mut mac = HmacSha256::new_from_slice(secret.as_ref().as_bytes())?;
		mac.update(query_string.as_bytes());
		let mac_bytes = mac.finalize().into_bytes();
		let signature = hex::encode(mac_bytes);

		let url = format!("{endpoint_str}?{query_string}&signature={signature}");
		let r = client.request(http_method.clone(), &url).send().await?;

		if r.status().is_success() {
			return Ok(r);
		}

		let error_html = r.text().await?; // assume it's html because we couldn't parse it into serde_json::Value
		if error_html.contains("<TITLE>ERROR: The request could not be satisfied</TITLE>") && attempt <= max_retries {
			if !encountered_cloudfront_error {
				tracing::warn!("Encountered CloudFront error. Oh boy, here we go again.");
				encountered_cloudfront_error = true;
			} else {
				tracing::debug!("CloudFront error encountered again. Attempting retry #{attempt} in {retry_delay:?}");
			}
			tokio::time::sleep(retry_delay).await;
			retry_delay += std::time::Duration::from_secs(1);
			continue;
		}

		return Err(unexpected_response_str(&error_html));
	}

	bail!("Max retries reached. Request failed.")
}
#[instrument]
pub async fn unsigned_request(http_method: reqwest::Method, endpoint_str: &str, params: HashMap<&str, String>) -> Result<reqwest::Response> {
	debug!("requesting unsigned\nEndpoint: {endpoint_str}\nParams: {:?}", &params);
	let client = reqwest::Client::new();
	let r = client.request(http_method, endpoint_str).query(&params).send().await?;

	if r.status().is_success() {
		return Ok(r);
	}

	let error_html = r.text().await?; // assume it's html because we couldn't parse it into serde_json::Value
	Err(unexpected_response_str(&error_html))
}
#[instrument(skip(key, secret))]
pub async fn get_balance(key: String, secret: String, market: Market) -> Result<f64> {
	let mut params = HashMap::<&str, String>::new();
	params.insert("recvWindow", "60000".to_owned());
	match market {
		Market::BinanceFutures => {
			let base_url = market.get_base_url();
			let url = base_url.join("fapi/v3/balance")?;

			let r = signed_request(reqwest::Method::GET, url.as_str(), params, key, secret).await?;
			let asset_balances: Vec<FuturesBalance> = deser_reqwest::<Vec<FuturesBalance>>(r).await?;

			let mut total_balance = 0.0;
			for asset in asset_balances {
				total_balance += asset.balance.parse::<f64>()?;
			}
			Ok(total_balance)
		}
		Market::BinanceSpot => {
			let base_url = market.get_base_url();
			let url = base_url.join("/api/v3/account")?;

			let r = signed_request(reqwest::Method::GET, url.as_str(), params, key, secret).await?;
			let account_details: SpotAccountDetails = deser_reqwest(r).await?;
			let asset_balances = account_details.balances;

			let mut total_balance = 0.0;
			for asset in asset_balances {
				total_balance += asset.free.parse::<f64>()?;
				total_balance += asset.locked.parse::<f64>()?;
			}
			Ok(total_balance)
		}
		Market::BinanceMargin => {
			let base_url = market.get_base_url();
			let url = base_url.join("/sapi/v1/margin/account")?;

			let r = signed_request(reqwest::Method::GET, url.as_str(), params, key, secret).await?;
			let account_details: MarginAccountDetails = deser_reqwest(r).await?;
			let total_balance: f64 = account_details.TotalCollateralValueInUSDT.parse()?;

			Ok(total_balance)
		}
	}
}
#[instrument]
pub async fn futures_price(asset: &str) -> Result<f64> {
	debug!("requesting futures price"); //doesn't flush immediately, needs fixing to be useful
	let symbol = crate::exchange_apis::Symbol {
		base: asset.to_string(),
		quote: "USDT".to_string(),
		market: Market::BinanceFutures,
	};
	let base_url = Market::BinanceFutures.get_base_url();
	let url = base_url.join("/fapi/v2/ticker/price")?;

	let mut params = HashMap::<&str, String>::new();
	params.insert("symbol", symbol.to_string());

	let r = unsigned_request(Method::GET, url.as_str(), params).await?;
	let price_response: PriceResponse = deser_reqwest(r).await?;

	Ok(price_response.price)
}
#[instrument]
pub async fn close_orders(key: String, secret: String, orders: &[BinanceOrder]) -> Result<()> {
	let base_url = Market::BinanceFutures.get_base_url();
	let url = base_url.join("/fapi/v1/order").unwrap();

	let handles = orders.iter().map(|o| {
		let mut params = HashMap::<&str, String>::new();
		params.insert("symbol", o.base_info.symbol.to_string());
		params.insert("orderId", o.binance_id.unwrap().to_string());
		params.insert("recvWindow", "60000".to_owned()); // dbg currently they are having some issues with response speed

		signed_request(reqwest::Method::DELETE, url.as_str(), params, key.clone(), secret.clone())
	});
	for handle in handles {
		let r = handle.await?;
		let _: CancelOrdersResponse = deser_reqwest(r).await?;
	}

	Ok(())
}
#[instrument(skip_all)]
pub async fn get_futures_positions(key: String, secret: String) -> Result<HashMap<String, f64>> {
	let url = FuturesAllPositionsResponse::get_url();

	let r = signed_request(Method::GET, url.as_str(), HashMap::new(), key, secret).await?;
	let positions: Vec<FuturesAllPositionsResponse> = deser_reqwest(r).await?;

	let mut positions_map = HashMap::<String, f64>::new();
	for position in positions {
		let symbol = position.symbol.clone();
		let qty = position.positionAmt.parse::<f64>()?;
		positions_map.entry(symbol).and_modify(|e| *e += qty).or_insert(qty);
	}
	Ok(positions_map)
}
#[instrument(skip(key, secret, binance_exchange_arc))]
pub async fn post_futures_order(key: String, secret: String, order: &Order<PositionOrderId>, binance_exchange_arc: Arc<RwLock<BinanceExchange>>) -> Result<BinanceOrder> {
	debug!("Posting order");
	let url = FuturesPositionResponse::get_url();

	let mut binance_order = BinanceOrder::from_standard(order.clone(), binance_exchange_arc).await;
	let mut params = binance_order.to_params();
	params.insert("recvWindow", "60000".to_owned()); // dbg currently they/me are having some issues with response speed

	let r = signed_request(reqwest::Method::POST, url.as_str(), params, key, secret).await?;
	let response: FuturesPositionResponse = deser_reqwest(r).await?;
	binance_order.binance_id = Some(response.order_id);
	Ok(binance_order)
}
/// Normally, the only cases where the return from this poll is going to be _reacted_ to, is when response.status == OrderStatus::Filled or an error is returned.
// TODO!: translate to websockets
#[instrument(skip(key, secret))]
pub async fn poll_futures_order<S: AsRef<str>>(key: S, secret: S, binance_order: &BinanceOrder) -> Result<FuturesPositionResponse> {
	let url = FuturesPositionResponse::get_url();

	let mut params = HashMap::<&str, String>::new();
	params.insert("symbol", binance_order.base_info.symbol.to_string());
	params.insert("orderId", format!("{}", &binance_order.binance_id.unwrap()));
	params.insert("recvWindow", "20000".to_owned()); // dbg currently they are having some issues with response speed
	debug!("Polling order");

	let r = signed_request(reqwest::Method::GET, url.as_str(), params, key, secret).await?;
	let response: FuturesPositionResponse = deser_reqwest(r).await?;
	Ok(response)
}
#[derive(Debug, Deserialize)]
pub struct BinanceKline {
	open_time: i64,
	open: String,
	high: String,
	low: String,
	close: String,
	volume: String,
	close_time: i64,
	quote_asset_volume: String,
	number_of_trades: i64,
	taker_buy_base_asset_volume: String,
	taker_buy_quote_asset_volume: String,
	ignore: String,
}
#[instrument]
pub async fn get_historic_klines(symbol: String, interval: String, limit: usize) -> Result<Vec<BinanceKline>> {
	let base_url = Market::BinanceFutures.get_base_url();
	let endpoint = base_url.join("/fapi/v1/klines")?;

	let params = vec![("symbol", symbol), ("interval", interval), ("limit", limit.to_string())];

	let response = unsigned_request(Method::GET, endpoint.as_str(), params.into_iter().collect()).await?;

	if !response.status().is_success() {
		let error_body = response.text().await?;
		bail!("Binance API error: {error_body}");
	}

	let klines: Vec<BinanceKline> = response.json().await?;
	Ok(klines)
}
/// NB: must be communicating back to the hub, can't shortcut and talk back directly to positions.
#[instrument(skip_all)]
pub async fn binance_runtime(
	live_settings: Arc<LiveSettings>,
	parent_js: &mut JoinSet<()>,
	hub_callback: mpsc::Sender<ExchangeToHub>,
	mut hub_rx: watch::Receiver<HubToExchange>,
	binance_exchange_arc: Arc<RwLock<BinanceExchange>>,
) {
	debug!("Binance_runtime started");
	let mut last_reported_fill_key = Uuid::default();
	let currently_deployed: Arc<RwLock<Vec<BinanceOrder>>> = Arc::new(RwLock::new(Vec::new()));

	use secrecy::ExposeSecret;
	use v_exchanges::ExchangeName;

	let config = live_settings.config().expect("Failed to load config");
	let binance_config = config.get_exchange(ExchangeName::Binance).expect("Binance exchange config not found");

	let pubkey = binance_config.api_pubkey.clone();
	let secret = binance_config.api_secret.expose_secret().to_string();

	let (temp_fills_stack_tx, mut temp_fills_stack_rx) = tokio::sync::mpsc::channel(100);
	let currently_deployed_clone = currently_deployed.clone();
	let (pubkey_clone, secret_clone) = (pubkey.clone(), secret.clone());

	// Polling orders for fills
	parent_js.spawn(async move {
		// TODO!!!: make into a websocket
		//LOOP: want to pull the orders for entire lifetime of the runtime. Later will be a websocket.
		loop {
			tokio::time::sleep(std::time::Duration::from_secs(5)).await;

			debug!("gonna request deployed orders. Could hang here.");
			let mut orders: Vec<_> = {
				let currently_deployed_read = currently_deployed_clone.read().unwrap();
				currently_deployed_read.iter().cloned().collect()
			};
			debug!("Local knowledge of deployed orders: {orders:?}");

			// Will update to websocket later, so requesting the actual deployed orders is free.

			// shuffle orders so there is no positional bias when polling
			let mut rng = SmallRng::from_rng(&mut rand::rng());
			orders.shuffle(&mut rng);

			for (i, order) in orders.iter().enumerate() {
				// // temp thing until I transfer to websocket
				let r: FuturesPositionResponse = match poll_futures_order(&pubkey_clone, &secret_clone, order).await {
					Ok(r) => r,
					Err(e) => {
						warn!("Error polling order: {e:?}, breaking to the outer order-pull task loop");
						continue;
					}
				};
				debug!("Successfully polled order: {r:?}");
				//

				// All other info except amount filled notional will only be relevant during trade's post-execution analysis.
				if r.executed_qty != order.notional_filled {
					{
						currently_deployed_clone.write().unwrap()[i].notional_filled = r.executed_qty;
					}
					temp_fills_stack_tx.send(FillFromPolling::new(order.base_info.clone(), r)).await.unwrap();
				}
			}
		}
	});

	// Keeping Exchange info up-to-date
	//TODO!: move to websockets, have them be right here.
	let binance_exchange_arc_clone = binance_exchange_arc.clone();
	let live_settings_clone = live_settings.clone();
	parent_js.spawn(async move {
		//LOOP: auxiliary information; can't halt the main loop
		loop {
			tokio::time::sleep(std::time::Duration::from_secs(15)).await;

			match BinanceExchangeFutures::init(live_settings_clone.clone()).await {
				Ok(binance_exchange_futures_updated) => {
					let mut binance_exchange_lock = binance_exchange_arc_clone.write().unwrap();
					binance_exchange_lock.binance_futures_info = binance_exchange_futures_updated;
				}
				Err(e) => {
					report_connection_problem(e.wrap_err("Error updating exchange info")).await;
				}
			}
		}
	});

	//LOOP: Main loop of Binance exchange
	loop {
		//dbg
		tokio::time::sleep(std::time::Duration::from_millis(100)).await;
		let now = chrono::Utc::now();
		println!("Binance runtime is still going: {}", now.format("%Y-%m-%d %H:%M:%S"));
		select! {
			Ok(_) = hub_rx.changed() => {
				handle_hub_orders_update(&hub_rx, &mut last_reported_fill_key, &pubkey, &secret, currently_deployed.clone(), binance_exchange_arc.clone()).await;
			},
			_ = handle_temp_fills_stack(&mut temp_fills_stack_rx, &hub_callback, &mut last_reported_fill_key, currently_deployed.clone()) => {},
		}
	}
}
#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub enum OrderStatus {
	#[default]
	#[serde(rename = "NEW")]
	New,
	#[serde(rename = "PARTIALLY_FILLED")]
	PartiallyFilled,
	#[serde(rename = "FILLED")]
	Filled,
	#[serde(rename = "CANCELED")]
	Canceled,
	#[serde(rename = "EXPIRED")]
	Expired,
	#[serde(rename = "EXPIRED_IN_MATCH")]
	ExpiredInMatch,
}
#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FuturesPositionResponse {
	pub client_order_id: Option<String>,
	pub cum_qty: Option<String>, // weird field, included at random (json api things)
	pub cum_quote: String,       // total filled quote asset
	#[serde_as(as = "DisplayFromStr")]
	pub executed_qty: f64, // total filled base asset
	pub order_id: i64,
	pub avg_price: Option<String>,
	pub orig_qty: String,
	pub price: String,
	pub reduce_only: Value,
	pub side: String,
	pub position_side: Option<String>, // only sent when in hedge mode
	pub status: OrderStatus,
	pub stop_price: String,
	pub close_position: Value,
	pub symbol: String,
	pub time_in_force: String,
	pub r#type: String,
	pub orig_type: String,
	pub activate_price: Option<f64>, // only returned on TRAILING_STOP_MARKET order
	pub price_rate: Option<f64>,     // only returned on TRAILING_STOP_MARKET order
	pub update_time: i64,
	pub working_type: Option<String>, // no clue what this is
	pub price_protect: bool,
	pub price_match: Option<String>, // huh
	pub self_trade_prevention_mode: Option<String>,
	pub good_till_date: Option<i64>,
}
impl FuturesPositionResponse {
	pub fn get_url() -> Url {
		let base_url = Market::BinanceFutures.get_base_url();
		// the way this works - is we sumbir "New" and "Query" to the same endpoint. The action is then determined by the presence of the orderId parameter.
		base_url.join("/fapi/v1/order").unwrap()
	}
}

mod orders;
use std::{
	collections::HashMap,
	sync::{Arc, RwLock},
};

use chrono::Utc;
use color_eyre::eyre::{Result, bail};
use hmac::{Hmac, Mac};
use info::BinanceExchangeFutures;
pub use orders::*;
use rand::{SeedableRng, rngs::SmallRng, seq::SliceRandom};
use reqwest::{
	Method,
	header::{CONTENT_TYPE, HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use serde_with::{DisplayFromStr, serde_as};
use sha2::Sha256;
use tokio::{
	select,
	sync::{mpsc, watch},
	task::JoinSet,
};
use tracing::{debug, instrument, warn};
use url::Url;
use uuid::Uuid;
use v_utils::{Percent, trades::Ohlc};

use super::{
	hub::{ExchangeToHub, HubToExchange},
	order_types::{ConceptualMarket, ConceptualOrderType, IdRequirements},
};
use crate::{
	MAX_CONNECTION_FAILURES, PositionOrderId,
	config::LiveSettings,
	exchange_apis::{Market, order_types::Order},
	utils::{deser_reqwest, report_connection_problem, unexpected_response_str},
};
type HmacSha256 = Hmac<Sha256>;

#[serde_as]
#[derive(Clone, Debug, Default, Deserialize, Serialize, derive_new::new)]
struct PriceResponse {
	#[serde_as(as = "DisplayFromStr")]
	price: f64,
	symbol: String,
	time: i64,
}

impl From<BinanceKline> for Ohlc {
	fn from(val: BinanceKline) -> Self {
		Ohlc {
			open: val.open.parse().unwrap(),
			high: val.high.parse().unwrap(),
			low: val.low.parse().unwrap(),
			close: val.close.parse().unwrap(),
		}
	}
}

#[derive(Clone, Debug, Default, derive_new::new)]
struct FillFromPolling {
	order: Order<PositionOrderId>,
	market_response: FuturesPositionResponse, //HACK: harcodes futures
}

#[instrument(skip(hub_callback))]
async fn handle_temp_fills_stack(
	temp_fills_stack_rx: &mut mpsc::Receiver<FillFromPolling>,
	hub_callback: &mpsc::Sender<ExchangeToHub>,
	last_reported_fill_key: &mut Uuid,
	currently_deployed: Arc<RwLock<Vec<BinanceOrder>>>,
) {
	while let Ok(f) = temp_fills_stack_rx.try_recv() {
		let new_fill_key = Uuid::now_v7();
		let r = f.market_response;

		if r.status == OrderStatus::Filled {
			let filled_id = &f.order.id;
			let mut deployed_lock = currently_deployed.write().unwrap();
			deployed_lock.retain(|o| o.base_info.id != *filled_id);
		}

		let callback = ExchangeToHub::new(new_fill_key, Market::BinanceFutures, r.executed_qty, f.order);
		debug!(?callback);
		hub_callback.send(callback).await.unwrap();
		*last_reported_fill_key = new_fill_key;
	}
}

#[instrument(skip(pubkey, secret, binance_exchange_arc))]
async fn handle_hub_orders_update(
	hub_rx: &watch::Receiver<HubToExchange>,
	last_reported_fill_key: &mut Uuid,
	pubkey: &str,
	secret: &str,
	currently_deployed: Arc<RwLock<Vec<BinanceOrder>>>,
	binance_exchange_arc: Arc<RwLock<BinanceExchange>>,
) {
	let target_orders: Vec<Order<PositionOrderId>>;
	{
		let from_hub = hub_rx.borrow();
		target_orders = if from_hub.key == *last_reported_fill_key {
			from_hub.orders.clone()
		} else {
			debug!("fill keys don't match.");
			return;
		};
	}
	debug!("fill keys match");

	// Close currently deployed orders
	//FUCK: will hang for 50s if re-requesting knowledge is only possible by continuing the execution of this exact function.
	//
	// Always will break naturally, this is only to prevent uncapped loops antipattern.
	for _ in 0..MAX_CONNECTION_FAILURES {
		let currently_deployed_clone;
		{
			currently_deployed_clone = currently_deployed.read().unwrap().clone();
		}
		match close_orders(pubkey.to_string(), secret.to_string(), &currently_deployed_clone).await {
			Ok(_) => break,
			Err(e) => {
				let inner_unexpected_response_str = e.chain().last().unwrap();
				if let Ok(error_value) = serde_json::from_str::<serde_json::Value>(&inner_unexpected_response_str.to_string())
					&& let Some(error_code) = error_value.get("code")
					&& error_code == -2011
				{
					let mut break_after = false;
					if report_connection_problem(e.wrap_err("Tried to close an order not existing on the remote: {:?}.\nWill wait 5s, try to lock_read the new value and try again. NB: Could loop forever if local knowledge is wrong and not just out of sync.")).await {
								break_after = true;
							}
					tokio::time::sleep(std::time::Duration::from_secs(5)).await;
					if break_after {
						break;
					};
				}
			}
		}
	}
	trace!("closed orders");

	let mut just_deployed = Vec::new();
	for o in target_orders {
		let b = match post_futures_order(pubkey.to_string(), secret.to_string(), &o, binance_exchange_arc.clone()).await {
			Ok(order) => order,
			Err(e) => {
				tracing::error!("Error posting order: {e:?}");
				continue;
			}
		};
		just_deployed.push(b);
	}
	info!(?just_deployed);

	{
		let mut current_lock = currently_deployed.write().unwrap();
		*current_lock = just_deployed;
	}
}

//=============================================================================
// Response structs {{{
//=============================================================================

#[derive(Debug, Deserialize, Serialize)]
struct FuturesBalance {
	accountAlias: String,
	asset: String,
	availableBalance: String,
	balance: String,
	crossUnPnl: String,
	crossWalletBalance: String,
	marginAvailable: bool,
	maxWithdrawAmount: String,
	updateTime: Number,
}

#[derive(Debug, Deserialize, Serialize)]
struct SpotAccountDetails {
	makerCommission: f64,
	takerCommission: f64,
	buyerCommission: f64,
	sellerCommission: f64,
	commissionRates: CommissionRates,
	canTrade: bool,
	canWithdraw: bool,
	canDeposit: bool,
	brokered: bool,
	requireSelfTradePrevention: bool,
	preventSor: bool,
	updateTime: u64,
	accountType: String,
	balances: Vec<SpotBalance>,
	permissions: Vec<String>,
	uid: u64,
}
#[derive(Debug, Deserialize, Serialize)]
struct CommissionRates {
	maker: String,
	taker: String,
	buyer: String,
	seller: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct SpotBalance {
	asset: String,
	free: String,
	locked: String,
}
#[derive(Debug, Deserialize, Serialize)]
struct MarginAccountDetails {
	borrowEnabled: bool,
	marginLevel: String,
	CollateralMarginLevel: String,
	totalAssetOfBtc: String,
	totalLiabilityOfBtc: String,
	totalNetAssetOfBtc: String,
	TotalCollateralValueInUSDT: String,
	tradeEnabled: bool,
	transferEnabled: bool,
	accountType: String,
	userAssets: Vec<MarginUserAsset>,
}

#[derive(Debug, Deserialize, Serialize)]
struct MarginUserAsset {
	asset: String,
	borrowed: String,
	free: String,
	interest: String,
	locked: String,
	netAsset: String,
}

#[serde_as]
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct FuturesSymbol {
	base_asset: String,
	base_asset_precision: u8,
	contract_type: String,
	delivery_date: i64,
	filters: Vec<Value>,
	#[serde_as(as = "DisplayFromStr")]
	liquidation_fee: f64,
	#[serde_as(as = "DisplayFromStr")]
	maint_margin_percent: f64,
	margin_asset: String,
	#[serde_as(as = "DisplayFromStr")]
	market_take_bound: f64,
	max_move_order_limit: Option<i64>,
	onboard_date: i64,
	order_types: Vec<String>,
	pair: String,
	price_precision: u8,
	quantity_precision: u8,
	quote_asset: String,
	quote_precision: u8,
	#[serde_as(as = "DisplayFromStr")]
	required_margin_percent: f64,
	settle_plan: Option<u32>,
	status: String,
	symbol: String,
	time_in_force: Vec<String>,
	#[serde_as(as = "DisplayFromStr")]
	trigger_protect: f64,
	underlying_sub_type: Vec<String>,
	underlying_type: String,
}

#[derive(Debug, Deserialize)]
struct FuturesAllPositionsResponse {
	entryPrice: String,
	breakEvenPrice: String,
	marginType: String,
	isAutoAddMargin: Value,
	isolatedMargin: String,
	leverage: String,
	liquidationPrice: String,
	markPrice: String,
	maxNotionalValue: String,
	positionAmt: String,
	notional: String,
	isolatedWallet: String,
	symbol: String,
	unRealizedProfit: String,
	positionSide: Value, // is "BOTH" in standard (non-hedge mode) requests, because designed by fucking morons. Apparently we now have negative values in `positionAmt`, if short.
	updateTime: i64,
}
impl FuturesAllPositionsResponse {
	pub fn get_url() -> Url {
		let base_url = Market::BinanceFutures.get_base_url();
		base_url.join("/fapi/v2/positionRisk").unwrap()
	}
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct CancelOrdersResponse {
	client_order_id: String,
	cum_qty: String,
	cum_quote: String,
	executed_qty: String,
	order_id: i64,
	orig_qty: String,
	orig_type: String,
	price: String,
	reduce_only: bool,
	side: String,
	position_side: String,
	status: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	stop_price: Option<String>,
	close_position: bool,
	symbol: String,
	time_in_force: String,
	#[serde(rename = "type")]
	order_type: String,
	#[serde(skip_serializing_if = "Option::is_none")]
	activate_price: Option<String>,
	#[serde(skip_serializing_if = "Option::is_none")]
	price_rate: Option<String>,
	update_time: i64,
	working_type: String,
	price_protect: bool,
	price_match: String,
	self_trade_prevention_mode: String,
	good_till_date: i64,
}
//,}}}
