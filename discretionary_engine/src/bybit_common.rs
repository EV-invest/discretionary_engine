use std::sync::Arc;

use color_eyre::eyre::{Context, Result};
use hmac::{Hmac, Mac};
use nautilus_bybit::http::client::{BybitHttpClient, BybitRawHttpClient};
use secrecy::ExposeSecret;
use sha2::Sha256;
use tracing::info;
use v_exchanges::ExchangeName;

use crate::config::LiveSettings;

pub fn convert_symbol_to_bybit(symbol: &str) -> String {
	let without_suffix = symbol.split('.').next().unwrap_or(symbol);
	without_suffix.replace('-', "").to_uppercase()
}

pub fn create_bybit_clients(live_settings: Arc<LiveSettings>, exchange_name: ExchangeName, testnet: bool) -> Result<(BybitRawHttpClient, BybitHttpClient)> {
	let config = live_settings.config()?;
	let exchange_config = config.get_exchange(exchange_name)?;

	let base_url = if testnet {
		info!("Using TESTNET");
		Some("https://api-testnet.bybit.com".to_string())
	} else {
		info!("Using MAINNET");
		None
	};

	let raw_client = BybitRawHttpClient::new(
		base_url.clone(),
		Some(60), // timeout_secs
		None,     // max_retries
		None,     // retry_delay_ms
		None,     // retry_delay_max_ms
		None,     // recv_window_ms
		None,     // proxy_url
	)
	.context("Failed to create Bybit raw HTTP client")?;

	let client = BybitHttpClient::with_credentials(
		exchange_config.api_pubkey.clone(),
		exchange_config.api_secret.expose_secret().to_string(),
		base_url,
		None, // timeout_secs
		None, // max_retries
		None, // retry_delay_ms
		None, // retry_delay_max_ms
		None, // recv_window_ms
		None, // proxy_url
	)
	.context("Failed to create Bybit HTTP client")?;

	Ok((raw_client, client))
}

/// Creates a new Bybit HTTP client wrapper that can call the amend endpoint
/// We need to store credentials to make authenticated requests
pub struct BybitAmendClient {
	api_key: String,
	api_secret: String,
	base_url: String,
	http_client: reqwest::Client,
}

impl BybitAmendClient {
	pub fn new(live_settings: Arc<LiveSettings>, exchange_name: ExchangeName, testnet: bool) -> Result<Self> {
		let config = live_settings.config()?;
		let exchange_config = config.get_exchange(exchange_name)?;

		let base_url = if testnet {
			"https://api-testnet.bybit.com".to_string()
		} else {
			"https://api.bybit.com".to_string()
		};

		Ok(Self {
			api_key: exchange_config.api_pubkey.clone(),
			api_secret: exchange_config.api_secret.expose_secret().to_string(),
			base_url,
			http_client: reqwest::Client::new(),
		})
	}

	/// Amend an order's price using orderLinkId
	pub async fn amend_order_by_link_id(&self, symbol: &str, order_link_id: &str, new_price: f64) -> Result<serde_json::Value> {
		let timestamp = chrono::Utc::now().timestamp_millis();
		let recv_window = 5000;

		let params = serde_json::json!({
			"category": "linear",
			"symbol": symbol,
			"orderLinkId": order_link_id,
			"price": format!("{}", new_price),
		});

		let param_str = serde_json::to_string(&params)?;

		// Bybit signature: timestamp + api_key + recv_window + param_str
		let sign_str = format!("{timestamp}{}{recv_window}{param_str}", self.api_key);

		let mut mac = Hmac::<Sha256>::new_from_slice(self.api_secret.as_bytes()).map_err(|e| color_eyre::eyre::eyre!("Invalid secret key: {e}"))?;
		mac.update(sign_str.as_bytes());
		let signature = hex::encode(mac.finalize().into_bytes());

		let url = format!("{}/v5/order/amend", self.base_url);

		let response = self
			.http_client
			.post(&url)
			.header("X-BAPI-API-KEY", &self.api_key)
			.header("X-BAPI-TIMESTAMP", timestamp.to_string())
			.header("X-BAPI-SIGN", signature)
			.header("X-BAPI-RECV-WINDOW", recv_window.to_string())
			.header("Content-Type", "application/json")
			.json(&params)
			.send()
			.await
			.context("Failed to send amend request")?;

		let response_text = response.text().await.context("Failed to read response")?;
		let response_json: serde_json::Value = serde_json::from_str(&response_text).context("Failed to parse response JSON")?;

		Ok(response_json)
	}

	/// Amend an order's price using orderId
	pub async fn amend_order_by_id(&self, symbol: &str, order_id: &str, new_price: f64) -> Result<serde_json::Value> {
		let timestamp = chrono::Utc::now().timestamp_millis();
		let recv_window = 5000;

		let params = serde_json::json!({
			"category": "linear",
			"symbol": symbol,
			"orderId": order_id,
			"price": format!("{}", new_price),
		});

		let param_str = serde_json::to_string(&params)?;

		// Bybit signature: timestamp + api_key + recv_window + param_str
		let sign_str = format!("{timestamp}{}{recv_window}{param_str}", self.api_key);

		let mut mac = Hmac::<Sha256>::new_from_slice(self.api_secret.as_bytes()).map_err(|e| color_eyre::eyre::eyre!("Invalid secret key: {e}"))?;
		mac.update(sign_str.as_bytes());
		let signature = hex::encode(mac.finalize().into_bytes());

		let url = format!("{}/v5/order/amend", self.base_url);

		let response = self
			.http_client
			.post(&url)
			.header("X-BAPI-API-KEY", &self.api_key)
			.header("X-BAPI-TIMESTAMP", timestamp.to_string())
			.header("X-BAPI-SIGN", signature)
			.header("X-BAPI-RECV-WINDOW", recv_window.to_string())
			.header("Content-Type", "application/json")
			.json(&params)
			.send()
			.await
			.context("Failed to send amend request")?;

		let response_text = response.text().await.context("Failed to read response")?;
		let response_json: serde_json::Value = serde_json::from_str(&response_text).context("Failed to parse response JSON")?;

		Ok(response_json)
	}
}
