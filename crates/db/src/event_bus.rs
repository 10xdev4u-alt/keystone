//! Cross-node event bus abstraction.
//!
//! Handlers depend on the [`EventBus`] trait, never on a concrete broker, so
//! a Redis/NATS/broker-backed implementation can replace [`PgNotifyBus`]
//! without touching application code. The concrete Postgres implementation
//! uses `LISTEN`/`NOTIFY`: a dedicated connection per channel relays
//! notifications into an in-process `broadcast` fan-out, which the API layer
//! wires into SSE / WebSocket subscribers.
//!
//! A notification published on any node is delivered to every subscriber of
//! that channel on every node running a listener — the shape the SSE hub
//! needs once the API scales past one instance.

use async_trait::async_trait;
use sqlx::PgPool;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::broadcast;

/// A single event relayed by the bus.
#[derive(Debug, Clone)]
pub struct BusEvent {
    /// Channel the event was published on (e.g. `notifications`, `chat`).
    pub channel: String,
    /// Raw payload; callers own the encoding (usually JSON).
    pub payload: String,
}

/// Error type for bus operations.
#[derive(Debug, thiserror::Error)]
pub enum BusError {
    #[error("publish failed: {0}")]
    Publish(#[from] sqlx::Error),
    #[error("listener failed: {0}")]
    Listener(String),
}

/// Event bus contract — pub/sub over named channels.
#[async_trait]
pub trait EventBus: Send + Sync + 'static {
    /// Publish `payload` on `channel`. Completes once Postgres has accepted
    /// the `NOTIFY` (durable to the point of hand-off to the broker).
    async fn publish(&self, channel: &str, payload: &str) -> Result<(), BusError>;

    /// Subscribe to a channel. Returns a broadcast receiver; events published
    /// before this call are not replayed (at-least-once delivery is the
    /// caller's job via id-cursors / Last-Event-ID).
    fn receiver(&self, channel: &str) -> broadcast::Receiver<BusEvent>;
}

/// Postgres `LISTEN`/`NOTIFY` bus.
///
/// One dedicated connection is held open per channel being listened to. The
/// channel set is demand-driven: the first `receiver(channel)` call spawns
/// that channel's listener. `publish` never waits on listeners — it fires the
/// `NOTIFY` and returns.
pub struct PgNotifyBus {
    pool: PgPool,
    channels: Mutex<HashMap<String, broadcast::Sender<BusEvent>>>,
}

impl PgNotifyBus {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            channels: Mutex::new(HashMap::new()),
        }
    }

    /// Ensure a listener task exists for `channel`; returns its sender.
    fn ensure_sender(&self, channel: &str) -> broadcast::Sender<BusEvent> {
        let mut map = self.channels.lock().expect("bus channel map poisoned");
        if let Some(tx) = map.get(channel) {
            return tx.clone();
        }
        let (tx, _rx) = broadcast::channel(1024);
        let pool = self.pool.clone();
        let channel = channel.to_string();
        let tx_fanout = tx.clone();
        let tx_return = tx.clone();
        map.insert(channel.clone(), tx);
        drop(map);

        tokio::spawn(async move {
            // The PgListener owns its connection for the lifetime of the task;
            // `connect_with` acquires a dedicated one from the pool.
            if let Err(e) = run_listener(pool, &channel, tx_fanout).await {
                tracing::error!(%channel, error = %e, "bus listener stopped");
            }
        });
        tx_return
    }
}

/// Run one listener connection: `LISTEN channel`, relay every notification.
async fn run_listener(
    pool: PgPool,
    channel: &str,
    tx: broadcast::Sender<BusEvent>,
) -> Result<(), BusError> {
    let listener = sqlx::postgres::PgListener::connect_with(&pool).await?;
    let mut listener = listener;
    listener.listen(channel).await?;
    loop {
        let notification = listener.recv().await?;
        let event = BusEvent {
            channel: channel.to_string(),
            payload: notification.payload().to_owned(),
        };
        // Broadcast errors (no receivers) are normal — drop.
        let _ = tx.send(event);
    }
}

#[async_trait]
impl EventBus for PgNotifyBus {
    async fn publish(&self, channel: &str, payload: &str) -> Result<(), BusError> {
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(channel)
            .bind(payload)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    fn receiver(&self, channel: &str) -> broadcast::Receiver<BusEvent> {
        self.ensure_sender(channel).subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bus_error_is_send() {
        fn assert_send<T: Send>() {}
        assert_send::<BusError>();
    }
}
