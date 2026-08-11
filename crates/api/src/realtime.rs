//! Realtime hub + notification feed.
//!
//! Architecture:
//!   - [`RealtimeHub`] is the in-process fan-out: one `broadcast` channel per
//!     user for the notification/activity feed, one per conversation for
//!     chat. Channels are created on demand and pruned when they lose all
//!     receivers.
//!   - The SSE feed is DB-backed gap recovery: every notification has a
//!     `BIGSERIAL` id, the client sends `Last-Event-ID`, and the handler
//!     replays `WHERE id > $cursor` before chaining the live stream. A
//!     lagged subscriber gets an explicit `resync` event instead of silently
//!     missing data.
//!   - Cross-node scale-out is reserved for the `EventBus` (LISTEN/NOTIFY in
//!     `keystone-db`): handlers publish to the hub directly (fast path); a
//!     bus→hub bridge is the wiring change when the API runs multi-instance.
//!
//! The write path is always REST/DB-first: notifications are inserted through
//! the repository (durable), THEN fanned out to the hub. The hub is never the
//! source of truth.

use crate::auth::{map_repo_error, AuthUser};
use crate::error::ApiError;
use crate::AppState;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::{self, StreamExt};
use keystone_db::repositories::notifications::{Notifications, PreferenceUpdate};
use serde::Deserialize;
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::collections::HashMap;
use std::convert::Infallible;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::broadcast;
use utoipa::ToSchema;
use uuid::Uuid;

/// An activity-feed event for one user (mirrors the `notifications` row).
#[derive(Debug, Clone)]
pub struct FeedEvent {
    pub id: i64,
    pub kind: String,
    pub payload: JsonValue,
}

/// In-process fan-out hub.
#[derive(Debug, Default)]
pub struct RealtimeHub {
    feeds: RwLock<HashMap<Uuid, broadcast::Sender<Arc<FeedEvent>>>>,
    /// Per-conversation chat broadcast (frames are serialized strings).
    chats: RwLock<HashMap<Uuid, broadcast::Sender<String>>>,
}

impl RealtimeHub {
    pub fn new() -> Self {
        Self::default()
    }

    /// Subscribe to a user's feed; the returned receiver misses nothing sent
    /// after this call (gap recovery before the cursor is the handler's job).
    pub fn subscribe_feed(&self, user_id: Uuid) -> broadcast::Receiver<Arc<FeedEvent>> {
        let mut feeds = self.feeds.write().expect("feed map poisoned");
        let sender = feeds
            .entry(user_id)
            .or_insert_with(|| broadcast::channel(1024).0)
            .clone();
        sender.subscribe()
    }

    /// Fan out to every current subscriber of the user's feed.
    pub fn publish_feed(&self, user_id: Uuid, event: FeedEvent) {
        if let Some(sender) = self.feeds.read().expect("feed map poisoned").get(&user_id) {
            let _ = sender.send(Arc::new(event));
        }
    }

    /// Drop a user's channel when the last receiver goes away.
    pub fn prune_feed(&self, user_id: Uuid) {
        let mut feeds = self.feeds.write().expect("feed map poisoned");
        if let Some(sender) = feeds.get(&user_id) {
            if sender.receiver_count() == 0 {
                feeds.remove(&user_id);
            }
        }
    }

    /// Subscribe to a conversation's chat channel (a WS connection does this
    /// after the upgrade so it receives the members' broadcast).
    pub fn subscribe_chat(&self, conversation_id: Uuid) -> broadcast::Receiver<String> {
        let mut chats = self.chats.write().expect("chat map poisoned");
        chats
            .entry(conversation_id)
            .or_insert_with(|| broadcast::channel(1024).0)
            .subscribe()
    }

    /// Fan a chat frame out to a conversation's subscribers.
    pub fn publish_chat(&self, conversation_id: Uuid, frame: String) {
        if let Some(sender) = self
            .chats
            .read()
            .expect("chat map poisoned")
            .get(&conversation_id)
        {
            let _ = sender.send(frame);
        }
    }
}

/// Persist a notification and fan it out to the user's feed subscribers.
/// Returns the created notification id. Central so every trigger shares one
/// path (audit-friendly, and the SSE feed + unread count stay in lockstep).
/// Description of a notification to create + fan out. Central so every
/// trigger shares one path (audit-friendly, and the SSE feed + unread count
/// stay in lockstep).
pub(crate) struct Notify<'a> {
    pub user_id: Uuid,
    pub kind: &'a str,
    pub actor_id: Option<Uuid>,
    pub entity_type: &'a str,
    pub entity_id: Option<Uuid>,
    pub payload: JsonValue,
}

/// Persist a notification and fan it out to the user's feed subscribers.
pub(crate) async fn notify(pool: &PgPool, hub: &RealtimeHub, n: Notify<'_>) {
    let repo = Notifications::new(pool.clone());
    let user_id = n.user_id;
    let kind = n.kind.to_string();
    let payload = n.payload.clone();
    match repo
        .create(&keystone_db::repositories::notifications::NewNotification {
            user_id: n.user_id,
            kind: n.kind,
            actor_id: n.actor_id,
            entity_type: n.entity_type,
            entity_id: n.entity_id,
            payload: n.payload,
        })
        .await
    {
        Ok(row) => {
            hub.publish_feed(
                user_id,
                FeedEvent {
                    id: row.id,
                    kind,
                    payload,
                },
            );
        }
        Err(e) => {
            tracing::warn!(%user_id, kind, error = %e, "notification create failed");
        }
    }
}

// ── SSE feed ────────────────────────────────────────────────────────────────

/// `GET /api/v1/notifications/feed` — Server-Sent Events.
///
/// Gap recovery: send `Last-Event-ID: <notification-id>`; everything newer is
/// replayed from the database before the live stream chains on.
/// Server-Sent Events stream of the caller's notifications.
#[utoipa::path(
    get,
    path = "/api/v1/notifications/feed",
    responses(
        (status = 200, description = "SSE notification stream", content_type = "text/event-stream"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn notifications_feed(
    State(state): State<AppState>,
    user: AuthUser,
    headers: HeaderMap,
) -> Response {
    let last_id = headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse::<i64>().ok());

    // Subscribe BEFORE the DB query so nothing published between the query
    // and the subscription is lost; the live stream dedups against the
    // highest replayed id (the replay is strictly ascending).
    let rx = state.realtime.subscribe_feed(user.user_id);
    let repo = Notifications::new(state.pool.clone());
    let replay = match last_id {
        Some(after) => repo
            .list_after(user.user_id, after)
            .await
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let watermark = replay.last().map(|n| n.id).unwrap_or(last_id.unwrap_or(0));
    let replay_stream = stream::iter(replay).map(|n| {
        Ok::<_, Infallible>(
            SseEvent::default()
                .id(n.id.to_string())
                .event(n.kind.clone())
                .json_data(n.payload)
                .expect("payload must serialize"),
        )
    });

    let live_stream = stream::unfold((rx, watermark), |(mut rx, mut watermark)| async move {
        loop {
            match rx.recv().await {
                Ok(event) if event.id > watermark => {
                    watermark = event.id;
                    return Some((
                        Ok::<_, Infallible>(
                            SseEvent::default()
                                .id(event.id.to_string())
                                .event(event.kind.clone())
                                .json_data(event.payload.clone())
                                .expect("payload must serialize"),
                        ),
                        (rx, watermark),
                    ));
                }
                // Already delivered in the replay — skip, don't duplicate.
                Ok(_) => continue,
                // Overrun: tell the client to reconnect with Last-Event-ID so
                // it replays the gap from the database — never silently drop.
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    return Some((
                        Ok::<_, Infallible>(
                            SseEvent::default()
                                .event("resync")
                                .data("replay-from-last-event-id"),
                        ),
                        (rx, watermark),
                    ));
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    Sse::new(replay_stream.chain(live_stream))
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keep-alive"),
        )
        .into_response()
}

// ── Notifications REST ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListQuery {
    pub before: Option<i64>,
    pub limit: Option<i64>,
}

/// The caller's notification list (paged).
#[utoipa::path(
    get,
    path = "/api/v1/notifications",
    responses(
        (status = 200, description = "Notifications", body = NotificationListResponse),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn list_notifications(
    State(state): State<AppState>,
    user: AuthUser,
    Query(q): Query<ListQuery>,
) -> Result<Json<NotificationListResponse>, ApiError> {
    let repo = Notifications::new(state.pool.clone());
    let items = repo
        .list(user.user_id, q.before, q.limit.unwrap_or(20))
        .await
        .map_err(map_repo_error)?;
    // Read flag derived from the per-user cursor in ONE query — items at or
    // below the cursor are read, so the unread count stays trivially derivable.
    // No state row yet = cursor 0 (nothing read). `fetch_optional`, not
    // `fetch_one` — an empty states table is normal for a fresh user.
    let cursor = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT read_cursor FROM notification_states WHERE user_id = $1",
    )
    .bind(user.user_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(ApiError::Database)?
    .flatten()
    .unwrap_or(0);
    let unread = repo
        .unread_count(user.user_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<NotificationView> = items
        .into_iter()
        .map(|n| NotificationView {
            id: n.id,
            kind: n.kind,
            actor_id: n.actor_id.map(|a| a.to_string()),
            entity_type: n.entity_type,
            entity_id: n.entity_id.map(|e| e.to_string()),
            payload: n.payload,
            created_at: n.created_at.to_rfc3339(),
            is_read: n.id <= cursor,
        })
        .collect();
    Ok(Json(NotificationListResponse {
        notifications: items,
        unread,
        read_cursor: cursor,
    }))
}

/// The caller's unread notification count.
#[utoipa::path(
    get,
    path = "/api/v1/notifications/unread-count",
    responses(
        (status = 200, description = "Unread count", body = UnreadCountResponse),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn unread_count(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<UnreadCountResponse>, ApiError> {
    let repo = Notifications::new(state.pool.clone());
    let count = repo
        .unread_count(user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(UnreadCountResponse { unread: count }))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkReadRequest {
    /// Mark everything up to this id as read. `null` = mark all.
    pub up_to: Option<i64>,
}

/// Mark notifications read (by id, or all when ids are empty).
#[utoipa::path(
    post,
    path = "/api/v1/notifications/read",
    operation_id = "notifications_mark_read",
    request_body = MarkReadRequest,
    responses(
        (status = 200, description = "Updated unread state", body = ReadReceiptResponse),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn mark_read(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<MarkReadRequest>,
) -> Result<Json<ReadReceiptResponse>, ApiError> {
    let repo = Notifications::new(state.pool.clone());
    let cursor = match req.up_to {
        Some(id) => {
            repo.mark_read(user.user_id, id)
                .await
                .map_err(map_repo_error)?;
            id
        }
        None => repo
            .mark_all_read(user.user_id)
            .await
            .map_err(map_repo_error)?,
    };
    let unread = repo
        .unread_count(user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(ReadReceiptResponse {
        read_cursor: cursor,
        unread,
    }))
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct ReadReceiptResponse {
    pub read_cursor: i64,
    pub unread: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PreferencesRequest {
    pub in_app: Option<bool>,
    pub digest: Option<bool>,
    pub email: Option<bool>,
    pub muted_kinds: Option<Vec<String>>,
    pub quiet_hours_start: Option<i16>,
    pub quiet_hours_end: Option<i16>,
}

/// A single notification in the feed, with read state derived from the
/// per-user cursor (items at or below the cursor are read).
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct NotificationView {
    pub id: i64,
    pub kind: String,
    pub actor_id: Option<String>,
    pub entity_type: String,
    pub entity_id: Option<String>,
    pub payload: JsonValue,
    pub created_at: String,
    pub is_read: bool,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct NotificationListResponse {
    pub notifications: Vec<NotificationView>,
    pub unread: i64,
    pub read_cursor: i64,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct UnreadCountResponse {
    pub unread: i64,
}

/// The caller's notification preferences — the typed contract for both the
/// GET and PUT handlers (kills the previous `Value`-shaped responses).
#[derive(Debug, serde::Serialize, ToSchema)]
pub struct NotificationPreferencesView {
    pub in_app: bool,
    pub digest: bool,
    pub email: bool,
    pub muted_kinds: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_hours_start: Option<i16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet_hours_end: Option<i16>,
}

#[derive(Debug, serde::Serialize, ToSchema)]
pub struct PreferencesResponse {
    pub preferences: NotificationPreferencesView,
}

/// The caller's notification preferences.
#[utoipa::path(
    get,
    path = "/api/v1/notifications/preferences",
    responses(
        (status = 200, description = "Preferences", body = PreferencesResponse),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn get_preferences(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<PreferencesResponse>, ApiError> {
    let repo = Notifications::new(state.pool.clone());
    let prefs = repo
        .get_preferences(user.user_id)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(PreferencesResponse {
        preferences: NotificationPreferencesView {
            in_app: prefs.in_app,
            digest: prefs.digest,
            email: prefs.email,
            muted_kinds: prefs.muted_kinds,
            quiet_hours_start: prefs.quiet_hours_start,
            quiet_hours_end: prefs.quiet_hours_end,
        },
    }))
}

/// Update the caller's notification preferences.
#[utoipa::path(
    put,
    path = "/api/v1/notifications/preferences",
    request_body = PreferencesRequest,
    responses(
        (status = 200, description = "Updated preferences", body = PreferencesResponse),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn update_preferences(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<PreferencesRequest>,
) -> Result<Json<PreferencesResponse>, ApiError> {
    let repo = Notifications::new(state.pool.clone());
    let prefs = repo
        .upsert_preferences(
            user.user_id,
            &PreferenceUpdate {
                in_app: req.in_app,
                digest: req.digest,
                email: req.email,
                muted_kinds: req.muted_kinds,
                quiet_hours_start: req.quiet_hours_start,
                quiet_hours_end: req.quiet_hours_end,
            },
        )
        .await
        .map_err(map_repo_error)?;
    Ok(Json(PreferencesResponse {
        preferences: NotificationPreferencesView {
            in_app: prefs.in_app,
            digest: prefs.digest,
            email: prefs.email,
            muted_kinds: prefs.muted_kinds,
            quiet_hours_start: prefs.quiet_hours_start,
            quiet_hours_end: prefs.quiet_hours_end,
        },
    }))
}
