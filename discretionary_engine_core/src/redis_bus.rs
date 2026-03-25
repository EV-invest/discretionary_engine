use color_eyre::eyre::{Result, WrapErr};
use redis::{AsyncCommands, Client, aio::MultiplexedConnection, streams::StreamReadOptions};
use serde::{Serialize, de::DeserializeOwned};

pub async fn connect(port: u16) -> Result<MultiplexedConnection> {
	let url = format!("redis://127.0.0.1:{port}/");
	let client = Client::open(url.as_str()).wrap_err("Failed to create Redis client")?;
	let conn = client
		.get_multiplexed_async_connection()
		.await
		.wrap_err_with(|| format!("Redis is not running on port {port}. Start it with: redis-server --port {port}"))?;
	Ok(conn)
}

pub async fn publish<T: Serialize>(conn: &mut MultiplexedConnection, stream_key: &str, consumer_group: &str, command: &T) -> Result<String> {
	require_active_consumer(conn, stream_key, consumer_group).await?;
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
		// Destroy and recreate the consumer group so we don't inherit pending messages from dead consumers.
		let _: redis::RedisResult<()> = conn.xgroup_destroy(stream_key, consumer_group).await;
		init_consumer_group(conn, stream_key, consumer_group).await?;
		Ok(Self {
			conn: conn.clone(),
			consumer_name,
			stream_key,
			consumer_group,
		})
	}

	pub async fn next<T: DeserializeOwned>(&mut self) -> Result<Option<T>> {
		let opts = StreamReadOptions::default().group(self.consumer_group, &self.consumer_name).count(1);

		let result: redis::RedisResult<redis::streams::StreamReadReply> = self.conn.xread_options(&[self.stream_key], &[">"], &opts).await;

		match result {
			Ok(reply) => {
				for stream_key in reply.keys {
					for entry in stream_key.ids {
						let id = entry.id.clone();
						if let Some(cmd) = entry.map.get("cmd") {
							let cmd_str: String = redis::FromRedisValue::from_redis_value_ref(cmd).unwrap_or_else(|e| panic!("unexpected Redis value for 'cmd': {cmd:?} (error: {e})"));
							let command: T = serde_json::from_str(&cmd_str).wrap_err_with(|| format!("Failed to deserialize command: {cmd_str}"))?;
							let _: () = self.conn.xack(self.stream_key, self.consumer_group, &[&id]).await?;
							return Ok(Some(command));
						}
					}
				}
				tokio::time::sleep(std::time::Duration::from_millis(100)).await;
				Ok(None)
			}
			Err(e) => Err(e).wrap_err(format!("Failed to read from Redis stream '{}' (group: {})", self.stream_key, self.consumer_group)),
		}
	}
}

/// Max idle time (ms) before a consumer is considered dead.
const CONSUMER_ALIVE_THRESHOLD_MS: i64 = 10_000;

async fn require_active_consumer(conn: &mut MultiplexedConnection, stream_key: &str, consumer_group: &str) -> Result<()> {
	let result: redis::RedisResult<redis::Value> = redis::cmd("XINFO").arg("CONSUMERS").arg(stream_key).arg(consumer_group).query_async(conn).await;

	let has_live_consumer = match result {
		Ok(redis::Value::Array(consumers)) => consumers.iter().any(|consumer| {
			// Each consumer is an array of [key, value, key, value, ...].
			// We look for "idle" followed by its millisecond value.
			let redis::Value::Array(fields) = consumer else { return false };
			let mut iter = fields.iter();
			while let Some(key) = iter.next() {
				let redis::Value::BulkString(k) = key else { continue };
				if k == b"idle" {
					if let Some(val) = iter.next() {
						let idle: i64 = redis::FromRedisValue::from_redis_value_ref(val).unwrap_or(i64::MAX);
						return idle < CONSUMER_ALIVE_THRESHOLD_MS;
					}
				}
			}
			false
		}),
		_ => false,
	};

	if has_live_consumer {
		Ok(())
	} else {
		let exe = std::env::current_exe()
			.ok()
			.and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
			.unwrap_or_else(|| env!("CARGO_PKG_NAME").to_owned());
		color_eyre::eyre::bail!("Service is not running. Start it with: `{exe} daemon`")
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
