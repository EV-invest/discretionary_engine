use std::sync::atomic::Ordering;

use color_eyre::eyre::{Report, Result, WrapErr};
use serde::de::DeserializeOwned;
use tracing::{error, warn};

use crate::{MAX_CONNECTION_FAILURES, MUT_CURRENT_CONNECTION_FAILURES};

pub fn format_eyre_chain_for_user(e: eyre::Report) -> String {
	let chain = e.chain().rev().collect::<Vec<_>>();
	let mut s = String::new();
	for (i, e) in chain.into_iter().enumerate() {
		if i > 0 {
			s.push('\n');
		}
		s.push_str("-> ");
		s.push_str(&e.to_string());
	}
	s
}

// Deser Reqwest {{{
/// Tracks the caller; once the max number of failures is reached, formats with all the callers that contributed, then sends a notification with `v_notify`
///
/// # Returns
/// `true` if the max number of failures is reached, `false` otherwise
///
/// # Dependencies
/// [v_notify](<https://crates.io/crates/v_notify>) locally installed
//TODO!: print the list of "contributors" to the failure
pub async fn report_connection_problem(e: Report) -> bool {
	let failures = MUT_CURRENT_CONNECTION_FAILURES.fetch_add(1, Ordering::Relaxed);
	warn!("Likely connection problem: {e:?}");

	if failures + 1 >= MAX_CONNECTION_FAILURES {
		error!("Reached max current connection failures ({MAX_CONNECTION_FAILURES})");

		MUT_CURRENT_CONNECTION_FAILURES.store(0, Ordering::Relaxed);
		return true;
	}

	false
}
/// Basically reqwest's `json()`, but prints the target's content on deserialization error.
pub async fn deser_reqwest<T: DeserializeOwned>(r: reqwest::Response) -> Result<T> {
	let text = r.text().await?;
	deser_reqwest_core(text)
}
/// Blocking [deser_reqwest]
pub fn deser_reqwest_blocking<T: DeserializeOwned>(r: reqwest::blocking::Response) -> Result<T> {
	let text = r.text()?;
	deser_reqwest_core(text)
}
pub fn unexpected_response_str(s: &str) -> eyre::Report {
	let s = match serde_json::from_str::<serde_json::Value>(s) {
		Ok(v) => serde_json::to_string_pretty(&v).unwrap(),
		Err(_) => s.to_owned(),
	};
	let report = v_utils::utils::report_msg(s);
	report.wrap_err("Unexpected API response")
}
fn deser_reqwest_core<T: DeserializeOwned>(text: String) -> Result<T> {
	match serde_json::from_str::<T>(&text) {
		Ok(deserialized) => Ok(deserialized),
		Err(e) => {
			let mut error_msg = e.to_string();
			if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(&text) {
				//let _ = std::panic::catch_unwind(|| {
				//	dbg!(&json_value["symbols"][0]);
				//});

				let mut jd = serde_json::Deserializer::from_str(&text);
				let r: Result<T, _> = serde_path_to_error::deserialize(&mut jd);
				if let Err(e) = r {
					error_msg = e.path().to_string();
				}
			}
			Err(unexpected_response_str(&text)).wrap_err_with(|| error_msg)
		}
	}
}

//,}}}
