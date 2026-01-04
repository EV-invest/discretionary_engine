//! Redis pub/sub for strategy commands.
//!
//! Uses Redis Streams for reliable message delivery between the CLI (submit)
//! and the running strategy (start).

use color_eyre::eyre::{Result, WrapErr};
use redis::{AsyncCommands, Client, aio::MultiplexedConnection, streams::StreamReadOptions};

/// Redis stream key for strategy commands.
pub const STREAM_KEY: &str = "discretionary_engine:strategy:commands";

/// Consumer group name.
pub const CONSUMER_GROUP: &str = "strategy_consumers";

/// Connect to Redis asynchronously.
pub async fn connect(port: u16) -> Result<MultiplexedConnection> {
	let url = format!("redis://127.0.0.1:{port}/");
	let client = Client::open(url.as_str()).wrap_err("Failed to create Redis client")?;
	let conn = client.get_multiplexed_async_connection().await.wrap_err("Failed to connect to Redis")?;
	Ok(conn)
}

/// Publish a command to the Redis stream.
///
/// Returns the stream entry ID.
pub async fn publish_command(conn: &mut MultiplexedConnection, command: &str) -> Result<String> {
	let id: String = conn.xadd(STREAM_KEY, "*", &[("cmd", command)]).await.wrap_err("Failed to publish command to Redis stream")?;
	Ok(id)
}

/// Initialize the consumer group (creates if not exists).
pub async fn init_consumer_group(conn: &mut MultiplexedConnection) -> Result<()> {
	// Try to create the group, ignore error if it already exists
	let result: redis::RedisResult<()> = conn.xgroup_create_mkstream(STREAM_KEY, CONSUMER_GROUP, "0").await;

	match result {
		Ok(()) => Ok(()),
		Err(e) => {
			// BUSYGROUP means group already exists, which is fine
			if e.to_string().contains("BUSYGROUP") {
				Ok(())
			} else {
				Err(e).wrap_err("Failed to create consumer group")
			}
		}
	}
}

/// Subscribe to commands from the Redis stream.
///
/// This is a blocking call that yields commands as they arrive.
/// Uses consumer groups for reliable delivery.
pub async fn subscribe_commands(conn: &mut MultiplexedConnection, consumer_name: &str) -> Result<CommandSubscriber> {
	init_consumer_group(conn).await?;
	Ok(CommandSubscriber {
		conn: conn.clone(),
		consumer_name: consumer_name.to_string(),
	})
}

/// Command subscriber that reads from Redis stream.
pub struct CommandSubscriber {
	conn: MultiplexedConnection,
	consumer_name: String,
}

impl CommandSubscriber {
	/// Read the next command from the stream.
	///
	/// Blocks until a command is available or timeout (5 seconds).
	/// Returns None on timeout.
	pub async fn next(&mut self) -> Result<Option<(String, String)>> {
		let opts = StreamReadOptions::default()
			.group(CONSUMER_GROUP, &self.consumer_name)
			.block(5000) // 5 second timeout
			.count(1);

		let result: redis::RedisResult<redis::streams::StreamReadReply> = self.conn.xread_options(&[STREAM_KEY], &[">"], &opts).await;

		match result {
			Ok(reply) => {
				for stream_key in reply.keys {
					for entry in stream_key.ids {
						let id = entry.id.clone();
						if let Some(cmd) = entry.map.get("cmd")
							&& let redis::Value::BulkString(bytes) = cmd
						{
							let cmd_str = String::from_utf8_lossy(bytes).to_string();
							// Acknowledge the message
							let _: () = self.conn.xack(STREAM_KEY, CONSUMER_GROUP, &[&id]).await?;
							return Ok(Some((id, cmd_str)));
						}
					}
				}
				Ok(None)
			}
			Err(e) => {
				// Timeout returns an error, treat as None
				if e.to_string().contains("timeout") {
					Ok(None)
				} else {
					Err(e).wrap_err("Failed to read from Redis stream")
				}
			}
		}
	}
}
