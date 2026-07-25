pub mod messages;

use self::messages::{PipelineEvent, QueueCommand};
use crate::config::Config;
use crate::domain::StageCommand;
use crate::error::{AutoForgeError, Result};
use dashmap::DashMap;
use lapin::{
    options::{
        BasicAckOptions, BasicGetOptions, BasicNackOptions, BasicPublishOptions,
        ExchangeDeclareOptions, QueueBindOptions, QueueDeclareOptions,
    },
    types::{AMQPValue, FieldTable, LongString},
    BasicProperties, Channel, Connection, ConnectionProperties,
};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{debug, info};
use uuid::Uuid;

pub use messages::{PipelineEvent as Event, QueueCommand as Command};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_CONNECT_ATTEMPTS: u32 = 15;
const EXCHANGE: &str = "autoforge";

struct PendingAck {
    delivery_tag: u64,
}

pub struct MessageQueue {
    publish_channel: Channel,
    consume_channel: Arc<Mutex<Channel>>,
    pub commands_stream: String,
    pub events_stream: String,
    commands_dlq: String,
    events_dlq: String,
    pub consumer_group: String,
    pending: Arc<Mutex<HashMap<String, PendingAck>>>,
    retries: Arc<DashMap<String, i64>>,
}

impl MessageQueue {
    pub async fn connect(config: &Config) -> Result<Arc<Self>> {
        let mut last_err = None;
        for attempt in 1..=MAX_CONNECT_ATTEMPTS {
            match Self::try_connect(config).await {
                Ok(mq) => return Ok(mq),
                Err(err) => {
                    if attempt < MAX_CONNECT_ATTEMPTS {
                        tracing::warn!(
                            attempt,
                            max = MAX_CONNECT_ATTEMPTS,
                            error = %err,
                            "rabbitmq connect failed; retrying in {CONNECT_RETRY_DELAY:?}"
                        );
                        tokio::time::sleep(CONNECT_RETRY_DELAY).await;
                    }
                    last_err = Some(err);
                }
            }
        }

        Err(last_err
            .unwrap_or_else(|| AutoForgeError::Queue("failed to connect to RabbitMQ".into())))
    }

    async fn try_connect(config: &Config) -> Result<Arc<Self>> {
        let connection = tokio::time::timeout(
            CONNECT_TIMEOUT,
            Connection::connect(&config.rabbitmq_url, ConnectionProperties::default()),
        )
        .await
        .map_err(|_| {
            AutoForgeError::Queue(format!(
                "timed out connecting to RabbitMQ at {} after {CONNECT_TIMEOUT:?}",
                config.rabbitmq_url
            ))
        })?
        .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        let publish_channel = connection
            .create_channel()
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        let consume_channel = connection
            .create_channel()
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        let commands_stream = config.queue_commands_stream.clone();
        let events_stream = config.queue_events_stream.clone();
        let commands_dlq = format!("{commands_stream}.dlq");
        let events_dlq = format!("{events_stream}.dlq");

        let mq = Arc::new(Self {
            publish_channel,
            consume_channel: Arc::new(Mutex::new(consume_channel)),
            commands_stream,
            events_stream,
            commands_dlq,
            events_dlq,
            consumer_group: config.queue_consumer_group.clone(),
            pending: Arc::new(Mutex::new(HashMap::new())),
            retries: Arc::new(DashMap::new()),
        });

        mq.ensure_topology().await?;
        Ok(mq)
    }

    pub async fn ping(&self) -> Result<()> {
        self.publish_channel
            .queue_declare(
                &self.commands_stream,
                QueueDeclareOptions {
                    passive: true,
                    ..QueueDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        Ok(())
    }

    async fn ensure_topology(&self) -> Result<()> {
        self.publish_channel
            .exchange_declare(
                EXCHANGE,
                lapin::ExchangeKind::Direct,
                ExchangeDeclareOptions {
                    durable: true,
                    ..ExchangeDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        for (queue, dlq) in [
            (&self.commands_stream, &self.commands_dlq),
            (&self.events_stream, &self.events_dlq),
        ] {
            self.declare_work_queue(queue, dlq).await?;
        }

        info!(
            commands = %self.commands_stream,
            events = %self.events_stream,
            exchange = EXCHANGE,
            "rabbitmq topology ready"
        );
        Ok(())
    }

    async fn declare_work_queue(&self, queue: &str, dlq: &str) -> Result<()> {
        let mut args = FieldTable::default();
        args.insert(
            "x-dead-letter-exchange".into(),
            AMQPValue::LongString(LongString::from(EXCHANGE)),
        );
        args.insert(
            "x-dead-letter-routing-key".into(),
            AMQPValue::LongString(LongString::from(dlq.to_string())),
        );

        self.publish_channel
            .queue_declare(
                queue,
                QueueDeclareOptions {
                    durable: true,
                    ..QueueDeclareOptions::default()
                },
                args,
            )
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        self.publish_channel
            .queue_declare(
                dlq,
                QueueDeclareOptions {
                    durable: true,
                    ..QueueDeclareOptions::default()
                },
                FieldTable::default(),
            )
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        for routing_key in [queue, dlq] {
            self.publish_channel
                .queue_bind(
                    routing_key,
                    EXCHANGE,
                    routing_key,
                    QueueBindOptions::default(),
                    FieldTable::default(),
                )
                .await
                .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        }

        Ok(())
    }

    pub async fn enqueue_command(&self, cmd: &StageCommand) -> Result<String> {
        let payload = serde_json::to_string(&QueueCommand::from(cmd.clone()))
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        self.publish(&self.commands_stream, &payload).await
    }

    pub async fn enqueue_commands(&self, cmds: &[StageCommand]) -> Result<()> {
        for cmd in cmds {
            self.enqueue_command(cmd).await?;
        }
        Ok(())
    }

    pub async fn publish_event(&self, event: &PipelineEvent) -> Result<String> {
        let payload =
            serde_json::to_string(event).map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        self.publish(&self.events_stream, &payload).await
    }

    async fn publish(&self, routing_key: &str, payload: &str) -> Result<String> {
        let message_id = Uuid::new_v4().to_string();
        let props = BasicProperties::default()
            .with_message_id(message_id.clone().into())
            .with_delivery_mode(2);

        self.publish_channel
            .basic_publish(
                EXCHANGE,
                routing_key,
                BasicPublishOptions::default(),
                payload.as_bytes(),
                props,
            )
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;

        debug!(%message_id, routing_key, "message published");
        Ok(message_id)
    }

    pub async fn read_commands(
        &self,
        consumer: &str,
        count: usize,
        block_ms: u64,
    ) -> Result<Vec<(String, QueueCommand)>> {
        self.read_queue(&self.commands_stream, consumer, count, block_ms)
            .await
    }

    pub async fn read_events(
        &self,
        consumer: &str,
        count: usize,
        block_ms: u64,
    ) -> Result<Vec<(String, PipelineEvent)>> {
        self.read_queue(&self.events_stream, consumer, count, block_ms)
            .await
    }

    async fn read_queue<T: serde::de::DeserializeOwned>(
        &self,
        queue: &str,
        consumer: &str,
        count: usize,
        block_ms: u64,
    ) -> Result<Vec<(String, T)>> {
        let _ = consumer;
        let mut out = Vec::new();
        let deadline = Instant::now() + Duration::from_millis(block_ms);

        while out.len() < count && Instant::now() < deadline {
            let delivery = {
                let channel = self.consume_channel.lock().await;
                channel
                    .basic_get(queue, BasicGetOptions { no_ack: false })
                    .await
                    .map_err(|e| AutoForgeError::Queue(e.to_string()))?
            };

            let Some(delivery) = delivery else {
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            };

            let msg_id = delivery
                .properties
                .message_id()
                .as_ref()
                .map(|s| s.to_string())
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let body = String::from_utf8_lossy(&delivery.data).to_string();

            match serde_json::from_str::<T>(&body) {
                Ok(parsed) => {
                    self.pending.lock().await.insert(
                        msg_id.clone(),
                        PendingAck {
                            delivery_tag: delivery.delivery_tag,
                        },
                    );
                    out.push((msg_id, parsed));
                }
                Err(e) => {
                    let channel = self.consume_channel.lock().await;
                    channel
                        .basic_nack(
                            delivery.delivery_tag,
                            BasicNackOptions {
                                requeue: false,
                                ..BasicNackOptions::default()
                            },
                        )
                        .await
                        .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
                    return Err(AutoForgeError::Queue(format!(
                        "invalid message payload on {queue}: {e}"
                    )));
                }
            }
        }

        Ok(out)
    }

    pub async fn ack_command(&self, id: &str) -> Result<()> {
        self.ack(id).await
    }

    pub async fn ack_event(&self, id: &str) -> Result<()> {
        self.ack(id).await
    }

    async fn ack(&self, id: &str) -> Result<()> {
        let pending =
            self.pending.lock().await.remove(id).ok_or_else(|| {
                AutoForgeError::Queue(format!("unknown message id for ack: {id}"))
            })?;

        let channel = self.consume_channel.lock().await;
        channel
            .basic_ack(pending.delivery_tag, BasicAckOptions::default())
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        Ok(())
    }

    pub async fn nack_for_requeue(&self, id: &str) -> Result<()> {
        let pending =
            self.pending.lock().await.remove(id).ok_or_else(|| {
                AutoForgeError::Queue(format!("unknown message id for nack: {id}"))
            })?;

        let channel = self.consume_channel.lock().await;
        channel
            .basic_nack(
                pending.delivery_tag,
                BasicNackOptions {
                    requeue: true,
                    ..BasicNackOptions::default()
                },
            )
            .await
            .map_err(|e| AutoForgeError::Queue(e.to_string()))?;
        Ok(())
    }

    /// RabbitMQ는 컨슈머 장애 시 미승인 메시지를 자동 재큐잉한다.
    pub async fn claim_stale_events(
        &self,
        _consumer: &str,
        _min_idle_ms: i64,
        _count: usize,
    ) -> Result<Vec<(String, PipelineEvent)>> {
        Ok(vec![])
    }

    pub async fn claim_stale_commands(
        &self,
        _consumer: &str,
        _min_idle_ms: i64,
        _count: usize,
    ) -> Result<Vec<(String, QueueCommand)>> {
        Ok(vec![])
    }

    pub async fn incr_retry(&self, stream: &str, id: &str) -> Result<i64> {
        let key = format!("{stream}:{id}");
        let mut entry = self.retries.entry(key).or_insert(0);
        *entry += 1;
        Ok(*entry)
    }

    pub async fn dead_letter(
        &self,
        stream: &str,
        id: &str,
        payload: &str,
        error: &str,
    ) -> Result<()> {
        let dlq = if stream == self.commands_stream {
            &self.commands_dlq
        } else {
            &self.events_dlq
        };
        let body = serde_json::json!({
            "original_id": id,
            "error": error,
            "data": payload,
        });
        self.publish(dlq, &body.to_string()).await?;
        Ok(())
    }
}
