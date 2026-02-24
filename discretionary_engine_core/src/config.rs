use secrecy::SecretString;
use v_utils::macros as v_macros;

#[derive(Clone, Debug, v_macros::MyConfigPrimitives)]
pub struct ExchangeConfig {
	pub api_pubkey: String,
	pub api_secret: SecretString,
	#[serde(default)]
	pub passphrase: Option<SecretString>,
}
