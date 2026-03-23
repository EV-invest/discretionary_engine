use std::{collections::HashMap, sync::OnceLock};

use secrecy::{ExposeSecret as _, SecretString};
use v_exchanges::{Exchange, ExchangeName, Instrument};
use v_utils::macros as v_macros;

#[derive(Clone, Debug, Default, v_macros::MyConfigPrimitives)]
pub struct ExchangeConfig {
	pub api_pubkey: String,
	pub api_secret: SecretString,
	#[serde(default)]
	pub passphrase: Option<SecretString>,
	#[serde(default)]
	pub instruments: Vec<Instrument>,
}
pub type ConfiguredExchanges = HashMap<String, ExchangeConfig>;

/// r[global.exchanges.static]: initialized once at startup, never changes. Restart to update.
static CONFIGURED_EXCHANGES: OnceLock<ConfiguredExchanges> = OnceLock::new();

/// Initialize the global exchange config. Panics if called twice.
pub fn init_exchanges(exchanges: ConfiguredExchanges) {
	CONFIGURED_EXCHANGES.set(exchanges).expect("exchanges already initialized");
}

/// Build authenticated exchange clients from the global config.
pub fn build_exchanges() -> Vec<(Box<dyn Exchange>, Vec<Instrument>)> {
	let config = CONFIGURED_EXCHANGES.get().expect("exchanges not initialized");
	config
		.iter()
		.map(|(name_str, exch_config)| {
			let name: ExchangeName = name_str.parse().unwrap_or_else(|_| panic!("unknown exchange: {name_str}"));
			let mut client = name.init_client();
			client.auth(exch_config.api_pubkey.clone(), exch_config.api_secret.expose_secret().to_string().into());
			(client, exch_config.instruments.clone())
		})
		.collect()
}
