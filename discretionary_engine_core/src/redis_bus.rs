use color_eyre::eyre::{Result, WrapErr};
use redis::{AsyncCommands, Client, aio::MultiplexedConnection, streams::StreamReadOptions};
use serde::{Serialize, de::DeserializeOwned};

pub async fn connect(port: u16) -> Result<MultiplexedConnection> {
	let url = format!("redis://127.0.0.1:{port}/");
	let client = Client::open(url.as_str()).wrap_err("Failed to create Redis client")?;
	let conn = client.get_multiplexed_async_connection().await.wrap_err("Failed to connect to Redis")?;
	Ok(conn)
}

pub async fn publish<T: Serialize>(conn: &mut MultiplexedConnection, stream_key: &str, command: &T) -> Result<String> {
	let payload = serde_json::to_string(command).wrap_err("Failed to serialize command")?;
	let id: String = conn.xadd(stream_key, "*", &[("cmd", &payload)]).await.wrap_err("Failed to publish to Redis stream")?;
	Ok(id)
}

pub struct StreamSubscriber {
	conn: MultiplexedConnection,
	consumer_name: String,
	stream_key: &'static str,
	consumer_group: &'static str,
}
impl StreamSubscriber {
	pub async fn new(conn: &mut MultiplexedConnection, stream_key: &'static str, consumer_group: &'static str, consumer_name: String) -> Result<Self> {
		init_consumer_group(conn, stream_key, consumer_group).await?;
		Ok(Self {
			conn: conn.clone(),
			consumer_name,
			stream_key,
			consumer_group,
		})
	}

	pub async fn next<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
		let opts = StreamReadOptions::default().group(self.consumer_group, &self.consumer_name).block(5000).count(1);

		let result: redis::RedisResult<redis::streams::StreamReadReply> = self.conn.xread_options(&[self.stream_key], &[">"], &opts).await;

		match result {
			Ok(reply) => {
				for stream_key in reply.keys {
					for entry in stream_key.ids {
						let id = entry.id.clone();
						if let Some(cmd) = entry.map.get("cmd")
							&& let redis::Value::BulkString(bytes) = cmd
						{
							let cmd_str = String::from_utf8_lossy(bytes);
							let command: T = serde_json::from_str(&cmd_str).wrap_err_with(|| format!("Failed to deserialize command: {cmd_str}"))?;
							let _: () = self.conn.xack(self.stream_key, self.consumer_group, &[&id]).await?;
							return Ok(Some(command));
						}
					}
				}
				Ok(None)
			}
			Err(e) =>
				if e.to_string().contains("timeout") {
					Ok(None)
				} else {
					Err(e).wrap_err("Failed to read from Redis stream")
				},
		}
	}
}

async fn init_consumer_group(conn: &mut MultiplexedConnection, stream_key: &str, consumer_group: &str) -> Result<()> {
	let result: redis::RedisResult<()> = conn.xgroup_create_mkstream(stream_key, consumer_group, "0").await;
	match result {
		Ok(()) => Ok(()),
		Err(e) =>
			if e.to_string().contains("BUSYGROUP") {
				Ok(())
			} else {
				Err(e).wrap_err("Failed to create consumer group")
			},
	}
}
