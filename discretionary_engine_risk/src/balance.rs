use std::{collections::HashMap, str::FromStr};

use color_eyre::eyre::{Result, eyre};
use de_core::config::ExchangeConfig;
use tracing::{error, instrument};
use v_exchanges::{
	adapters::generics::http::{ApiError, AuthError, HandleError, RequestError},
	core::{Exchange, ExchangeName, Instrument},
	error::ExchangeError,
};
use v_utils::{trades::Usd, utils::Sysexit};

#[instrument]
pub async fn main(exchanges: &HashMap<String, ExchangeConfig>, other_balances: Option<&HashMap<String, f64>>) -> Result<(), RiskError> {
	let exchanges = initialize_exchanges(exchanges)?;
	let balances = collect_balances(&exchanges).await?;

	let mut sorted_balances: Vec<_> = balances.iter().collect();
	sorted_balances.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));

	for (key, balance) in sorted_balances {
		println!("{key}: {balance}$");
	}

	let other_sum = other_balances.map(|m| m.values().sum::<f64>()).filter(|s| *s != 0.0);
	if let Some(sum) = other_sum {
		println!("other: {sum}$");
	}

	let total_balance = get_total_balance(&balances, other_balances);
	println!("\nTotal: {total_balance}$");
	Ok(())
}
#[derive(Debug, thiserror::Error)]
pub enum RiskError {
	#[error("{0}")]
	Unauthorized(String),
	#[error(transparent)]
	Other(#[from] color_eyre::eyre::Report),
}
impl v_utils::utils::SysexitCode for RiskError {
	fn sysexit(&self) -> Sysexit {
		match self {
			RiskError::Unauthorized(_) => Sysexit::NoPerm,
			RiskError::Other(_) => Sysexit::default(),
		}
	}
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
pub async fn collect_balances(exchanges: &[Box<dyn Exchange>]) -> Result<HashMap<String, Usd>, RiskError> {
	let mut balances = HashMap::new();
	let mut unauthorized = Vec::new();
	for exchange in exchanges {
		let name = exchange.name().to_string();
		match exchange.balances(Instrument::Perp, None).await {
			Ok(balance) => {
				balances.insert(name, balance.total);
			}
			Err(e) if is_unauthorized(&e) => {
				error!("{name}: {e}");
				unauthorized.push(name);
			}
			Err(e) => {
				error!("{name}: {e}");
			}
		}
	}
	if !unauthorized.is_empty() {
		return Err(RiskError::Unauthorized(format!("Unauthorized: {}", unauthorized.join(", "))));
	}
	Ok(balances)
}
#[instrument(skip_all)]
pub fn get_total_balance(balances: &HashMap<String, Usd>, other_balances: Option<&HashMap<String, f64>>) -> Usd {
	let mut total = Usd(0.);
	for balance in balances.values() {
		total += *balance;
	}
	if let Some(others) = other_balances {
		total = Usd(*total + others.values().sum::<f64>());
	}
	total
}
fn is_unauthorized(e: &ExchangeError) -> bool {
	matches!(
		e,
		ExchangeError::Request(RequestError::HandleResponse(HandleError::Api(ApiError::Auth(AuthError::Unauthorized { .. }))))
	)
}
