//! Individual exchange APIs expose methods and frameworks for interacting with their respective exchanges. At this level we have `hub` and `exchanges` which interpret the information passed up by the individual exchanges in the manner necessary for the task. `exchanges` exposes information for Protocols and Positions, `hub` uses it to construct the optimal execution strategy.

pub mod binance;
pub mod exchanges;
pub mod hub;
pub mod order_types;

use color_eyre::eyre::{Result, bail};
use serde::{Deserialize, Serialize};
use url::Url;
use v_utils::macros::graphemics;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum Market {
	#[default]
	BinanceFutures,
	BinanceSpot,
	BinanceMargin,
}
impl Market {
	pub fn get_base_url(&self) -> Url {
		match self {
			Market::BinanceFutures => Url::parse("https://fapi.binance.com/").unwrap(),
			Market::BinanceSpot => Url::parse("https://api.binance.com/").unwrap(),
			Market::BinanceMargin => Url::parse("https://api.binance.com/").unwrap(),
		}
	}

	pub fn format_symbol(&self, symbol: &str) -> String {
		match self {
			Market::BinanceFutures => symbol.to_owned().to_uppercase() + "USDT",
			Market::BinanceSpot => symbol.to_owned().to_uppercase() + "USDT",
			Market::BinanceMargin => symbol.to_owned().to_uppercase() + "USDT",
		}
	}
}

impl std::str::FromStr for Market {
	type Err = eyre::Report;

	fn from_str(s: &str) -> Result<Self> {
		match s {
			_ if graphemics!(BinanceFutures).contains(&s) => Ok(Market::BinanceFutures),
			_ if graphemics!(BinanceSpot).contains(&s) => Ok(Market::BinanceSpot),
			_ if graphemics!(BinanceMargin).contains(&s) => Ok(Market::BinanceMargin),
			_ => bail!("Unknown market: {s}"),
		}
	}
}

/// Contains information sufficient to identify the exact orderbook.
/// ```rust
/// let symbol = "BTC-USDT-BinanceFutures".parse::<discretionary_engine::exchange_apis::Symbol>().unwrap();
/// ```
#[derive(Clone, Debug, Default, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Symbol {
	pub base: String,
	pub quote: String,
	pub market: Market,
}
impl Symbol {
	pub fn new<T: AsRef<str>>(base: T, quote: T, market: Market) -> Self {
		let base = base.as_ref().to_string();
		let quote = quote.as_ref().to_string();
		Self { base, quote, market }
	}

	pub fn ticker(&self) -> String {
		format!("{}{}", self.base, self.quote)
	}
}
impl std::fmt::Display for Symbol {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "{}", self.market.format_symbol(&self.base))
	}
}
impl std::str::FromStr for Symbol {
	type Err = eyre::Report;

	fn from_str(s: &str) -> Result<Self> {
		let split = s.split('-').collect::<Vec<&str>>();
		Ok(Self {
			base: split.get(0).unwrap().to_string(),
			quote: split.get(1).unwrap().to_string(),
			market: Market::from_str(split.get(2).unwrap())?,
		})
	}
}
