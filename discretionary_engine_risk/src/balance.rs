use std::{collections::HashMap, str::FromStr};

use color_eyre::eyre::{Result, eyre};
use discretionary_engine_core::config::ExchangeConfig;
use tracing::{error, instrument};
use v_exchanges::core::{Exchange, ExchangeName, Instrument};
use v_utils::trades::Usd;

#[instrument]
pub async fn main(exchanges: &HashMap<String, ExchangeConfig>, other_balances: Option<f64>) -> Result<()> {
	let exchanges = initialize_exchanges(exchanges)?;
	let balances = collect_balances(&exchanges).await?;

	let mut sorted_balances: Vec<_> = balances.iter().collect();
	sorted_balances.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));

	for (key, balance) in sorted_balances {
		println!("{key}: {balance}$"); // flush prematurely, as networking is the bottleneck. So showing faster is noticeable.
	}

	let total_balance = get_total_balance(&balances, other_balances);
	println!("\nTotal: {total_balance}$");
	Ok(())
}

#[instrument(skip_all)]
pub fn initialize_exchanges(exchanges: &HashMap<String, ExchangeConfig>) -> Result<Vec<Box<dyn Exchange>>> {
	if exchanges.is_empty() {
		tracing::warn!("No exchanges configured");
	}
	let mut out: Vec<Box<dyn Exchange>> = Vec::new();
	for (name, auth) in exchanges {
		let exchange_name = ExchangeName::from_str(name)?;
		let mut exchange = exchange_name.init_client();
		exchange.auth(auth.api_pubkey.clone(), auth.api_secret.clone());
		exchange.set_max_tries(3);
		exchange.set_recv_window(std::time::Duration::from_secs(15));

		if exchange_name == ExchangeName::Kucoin {
			let passphrase = auth.passphrase.clone().ok_or_else(|| eyre!("Kucoin exchange requires passphrase in config"))?;
			exchange.update_default_option(v_exchanges::kucoin::KucoinOption::Passphrase(passphrase));
		}

		out.push(exchange);
	}
	Ok(out)
}

#[instrument(skip_all)]
pub async fn collect_balances(exchanges: &[Box<dyn Exchange>]) -> Result<HashMap<String, Usd>> {
	let mut balances = HashMap::new();
	for exchange in exchanges {
		let name = exchange.name().to_string();
		match exchange.balances(Instrument::Perp, None).await {
			Ok(balance) => {
				balances.insert(name, balance.total);
			}
			Err(e) => {
				error!("{name}: {e}");
			}
		}
	}
	Ok(balances)
}

#[instrument(skip_all)]
pub fn get_total_balance(balances: &HashMap<String, Usd>, other_balances: Option<f64>) -> Usd {
	let mut total = Usd(0.);
	for balance in balances.values() {
		total += *balance;
	}
	if let Some(other) = other_balances {
		total = Usd(*total + other);
	}
	total
}
