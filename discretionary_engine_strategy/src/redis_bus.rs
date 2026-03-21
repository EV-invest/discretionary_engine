//! Redis pub/sub for strategy commands.
//!
//! Uses Redis Streams for reliable message delivery between the CLI (submit)
//! and the running strategy (start).

pub use de_core::redis_bus::connect;

pub const STREAM_KEY: &str = "discretionary_engine:strategy:commands";
pub const CONSUMER_GROUP: &str = "strategy_consumers";

/// Publish a command string to the strategy Redis stream.
pub async fn publish_command(conn: &mut redis::aio::MultiplexedConnection, command: &str) -> color_eyre::eyre::Result<String> {
	// Strategy still uses raw strings; wrap in a String for serde compat
	de_core::redis_bus::publish(conn, STREAM_KEY, &command).await
}

/// Subscribe to commands from the strategy Redis stream.
pub async fn subscribe_commands(conn: &mut redis::aio::MultiplexedConnection, consumer_name: &str) -> color_eyre::eyre::Result<de_core::redis_bus::StreamSubscriber> {
	de_core::redis_bus::StreamSubscriber::new(conn, STREAM_KEY, CONSUMER_GROUP, consumer_name.to_string()).await
}
