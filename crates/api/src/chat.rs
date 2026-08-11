//! Chat: REST write path + WebSocket transport.
//!
//! Messages are ALWAYS persisted through the repository (the normal write
//! path); the WebSocket is a thin transport that adds delivery acks, typing
//! and presence. Authorization is membership: the handshake verifies the
//! user is a conversation member BEFORE upgrading, and every REST read is
//! membership-gated at the repository. Message rate caps are enforced per
//! connection with a sliding window ([`MessageGate`]).

use crate::auth::{map_repo_error, AuthUser};
use crate::error::ApiError;
use crate::realtime::notify;
use crate::AppState;
use axum::extract::ws::{Message as WsMessage, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use futures::{SinkExt, StreamExt};
use keystone_db::repositories::chat::Chat;
use serde::Deserialize;
use serde_json::{json, Value as JsonValue};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use utoipa::ToSchema;
use uuid::Uuid;

/// Sliding-window rate gate for WS message sends.
#[derive(Debug)]
pub struct MessageGate {
    max: usize,
    window: Duration,
    history: Mutex<VecDeque<Instant>>,
}

impl MessageGate {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            max,
            window,
            history: Mutex::new(VecDeque::new()),
        }
    }

    /// `true` when the caller may send another message within the window.
    pub fn allow(&self) -> bool {
        let now = Instant::now();
        let mut history = self.history.lock().expect("gate poisoned");
        while history
            .front()
            .is_some_and(|t| now.duration_since(*t) > self.window)
        {
            history.pop_front();
        }
        if history.len() >= self.max {
            return false;
        }
        history.push_back(now);
        true
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateConversationRequest {
    #[serde(rename = "type")]
    pub kind: String,
    /// Direct: the other user's id.
    pub user_id: Option<Uuid>,
    /// Group: title + members.
    pub title: Option<String>,
    pub member_ids: Option<Vec<Uuid>>,
}

/// Create (or fetch) a direct-message conversation with another user.
#[utoipa::path(
    post,
    path = "/api/v1/conversations",
    request_body = CreateConversationRequest,
    responses(
        (status = 201, description = "Conversation created or existing", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn create_conversation(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<CreateConversationRequest>,
) -> Result<axum::response::Response, ApiError> {
    let chat = Chat::new(state.pool.clone());
    let (conversation, is_new) = match req.kind.as_str() {
        "direct" => {
            let other = req
                .user_id
                .ok_or_else(|| ApiError::BadRequest("user_id required for direct".into()))?;
            let conversation = chat
                .find_or_create_direct(user.user_id, other)
                .await
                .map_err(map_repo_error)?;
            (conversation, false)
        }
        "group" => {
            let title = req
                .title
                .ok_or_else(|| ApiError::BadRequest("title required for group".into()))?;
            let members = req.member_ids.unwrap_or_default();
            let conversation = chat
                .create_group(user.user_id, &title, &members)
                .await
                .map_err(map_repo_error)?;
            (conversation, true)
        }
        other => {
            return Err(ApiError::BadRequest(format!(
                "unknown conversation type: {other}"
            )))
        }
    };
    let status = if is_new {
        axum::http::StatusCode::CREATED
    } else {
        axum::http::StatusCode::OK
    };
    Ok((
        status,
        Json(json!({
            "conversation": {
                "id": conversation.id.to_string(),
                "type": conversation.kind,
                "title": conversation.title,
            }
        })),
    )
        .into_response())
}

/// The caller's conversations (latest message first).
#[utoipa::path(
    get,
    path = "/api/v1/conversations",
    responses(
        (status = 200, description = "Conversations", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn list_conversations(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<JsonValue>, ApiError> {
    let chat = Chat::new(state.pool.clone());
    let rows = chat
        .list_for_user(user.user_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<JsonValue> = rows
        .into_iter()
        .map(|c| {
            json!({
                "id": c.id.to_string(),
                "type": c.kind,
                "title": c.title,
                "created_at": c.created_at,
                "last_message_at": c.last_message_at,
                "last_message": c.last_message,
                "unread": c.unread,
            })
        })
        .collect();
    Ok(Json(json!({ "conversations": items })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MessagesQuery {
    pub before: Option<DateTime<Utc>>,
    pub limit: Option<i64>,
}

/// Messages in a conversation (cursor-paged).
#[utoipa::path(
    get,
    path = "/api/v1/conversations/{id}/messages",
    params(("id" = Uuid, Path, description = "Conversation id")),
    responses(
        (status = 200, description = "Messages", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn list_messages(
    State(state): State<AppState>,
    user: AuthUser,
    Path(conversation_id): Path<Uuid>,
    Query(q): Query<MessagesQuery>,
) -> Result<Json<JsonValue>, ApiError> {
    let chat = Chat::new(state.pool.clone());
    let rows = chat
        .list_messages(
            conversation_id,
            user.user_id,
            q.before,
            q.limit.unwrap_or(50),
        )
        .await
        .map_err(map_repo_error)?;
    let items: Vec<JsonValue> = rows
        .into_iter()
        .map(|m| {
            json!({
                "id": m.id.to_string(),
                "conversation_id": m.conversation_id.to_string(),
                "sender_id": m.sender_id.to_string(),
                "body": m.body,
                "sent_at": m.sent_at,
                "delivered_at": m.delivered_at,
                "read_at": m.read_at,
            })
        })
        .collect();
    Ok(Json(json!({ "messages": items })))
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SendMessageRequest {
    pub body: String,
}

/// REST write path — identical semantics to the WS path (persist first, then
/// fan out), so clients without a socket can still participate.
/// Send a message to a conversation (fans out over websockets).
#[utoipa::path(
    post,
    path = "/api/v1/conversations/{id}/messages",
    request_body = SendMessageRequest,
    params(("id" = Uuid, Path, description = "Conversation id")),
    responses(
        (status = 201, description = "Message sent", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn send_message(
    State(state): State<AppState>,
    user: AuthUser,
    Path(conversation_id): Path<Uuid>,
    Json(req): Json<SendMessageRequest>,
) -> Result<axum::response::Response, ApiError> {
    let chat = Chat::new(state.pool.clone());
    let message = chat
        .send_message(conversation_id, user.user_id, &req.body)
        .await
        .map_err(map_repo_error)?;
    fan_out_message(&state, &chat, &message, user.user_id).await;
    Ok((
        axum::http::StatusCode::CREATED,
        Json(json!({
            "message": {
                "id": message.id.to_string(),
                "conversation_id": message.conversation_id.to_string(),
                "sender_id": message.sender_id.to_string(),
                "body": message.body,
                "sent_at": message.sent_at,
            }
        })),
    )
        .into_response())
}

/// Mark a conversation read by the caller.
#[utoipa::path(
    post,
    path = "/api/v1/conversations/{id}/read",
    params(("id" = Uuid, Path, description = "Conversation id")),
    responses(
        (status = 200, description = "Unread state", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn mark_read(
    State(state): State<AppState>,
    user: AuthUser,
    Path(conversation_id): Path<Uuid>,
) -> Result<Json<JsonValue>, ApiError> {
    let chat = Chat::new(state.pool.clone());
    chat.mark_read(conversation_id, user.user_id)
        .await
        .map_err(map_repo_error)?;
    let unread = chat
        .list_for_user(user.user_id)
        .await
        .map_err(map_repo_error)?
        .iter()
        .find(|c| c.id == conversation_id)
        .map(|c| c.unread)
        .unwrap_or(0);
    Ok(Json(
        json!({ "conversation_id": conversation_id.to_string(), "unread": unread }),
    ))
}

/// Online presence for a conversation's participants.
#[utoipa::path(
    get,
    path = "/api/v1/conversations/{id}/presence",
    params(("id" = Uuid, Path, description = "Conversation id")),
    responses(
        (status = 200, description = "Presence map", body = Value),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn conversation_presence(
    State(state): State<AppState>,
    user: AuthUser,
    Path(conversation_id): Path<Uuid>,
) -> Result<Json<JsonValue>, ApiError> {
    let chat = Chat::new(state.pool.clone());
    // Membership is the privacy boundary — outsiders get 404 (existence
    // never confirmed), mirroring the block semantics elsewhere.
    if !chat
        .is_member(conversation_id, user.user_id)
        .await
        .map_err(map_repo_error)?
    {
        return Err(ApiError::NotFound);
    }
    let presence = chat
        .presence_for(conversation_id)
        .await
        .map_err(map_repo_error)?;
    let items: Vec<JsonValue> = presence
        .into_iter()
        .map(|p| {
            json!({
                "user_id": p.user_id.to_string(),
                "status": p.status,
                "last_seen_at": p.last_seen_at,
            })
        })
        .collect();
    Ok(Json(json!({ "presence": items })))
}

// ── WebSocket ───────────────────────────────────────────────────────────────

/// `GET /api/v1/ws/chat/{conversation_id}`.
///
/// Auth + membership are checked BEFORE the upgrade — a non-member never
/// reaches the socket.
/// Upgrade to the chat websocket for a conversation.
#[utoipa::path(
    get,
    path = "/api/v1/ws/chat/{id}",
    params(("id" = Uuid, Path, description = "Conversation id")),
    responses(
        (status = 101, description = "Websocket upgraded"),
        (status = 401, description = "Missing or invalid access token"),
    ),
    security(("bearer_auth" = [])),
    tag = "realtime"
)]
pub async fn chat_socket(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
    user: AuthUser,
    Path(conversation_id): Path<Uuid>,
) -> Result<Response, ApiError> {
    let chat = Chat::new(state.pool.clone());
    if !chat
        .is_member(conversation_id, user.user_id)
        .await
        .map_err(map_repo_error)?
    {
        return Err(ApiError::Forbidden);
    }
    let gate = std::sync::Arc::new(MessageGate::new(30, Duration::from_secs(10)));
    Ok(ws.on_upgrade(move |socket| handle_socket(socket, state, user, conversation_id, gate)))
}

/// Client→server frame shapes.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Message { body: String },
    Typing,
    Read,
    Ack { up_to: Option<DateTime<Utc>> },
}

/// Server→client frame shapes.
fn server_frame(kind: &str, conversation_id: Uuid, payload: JsonValue) -> String {
    json!({
        "type": kind,
        "conversation_id": conversation_id.to_string(),
        "payload": payload,
    })
    .to_string()
}

async fn handle_socket(
    socket: WebSocket,
    state: AppState,
    user: AuthUser,
    conversation_id: Uuid,
    gate: std::sync::Arc<MessageGate>,
) {
    let chat = Chat::new(state.pool.clone());
    let user_id = user.user_id;

    // Presence: online for the conversation's members, in DB + chat channel.
    let _ = chat.set_presence(user_id, "online").await;
    broadcast_chat(
        &state,
        conversation_id,
        "presence",
        json!({ "user_id": user_id.to_string(), "status": "online" }),
    );

    // Subscribe to the conversation channel so the members' events reach us.
    let mut rx = state.realtime.subscribe_chat(conversation_id);
    let (mut sink, mut stream) = socket.split();

    loop {
        tokio::select! {
            incoming = stream.next() => {
                match incoming {
                    Some(Ok(WsMessage::Text(text))) => {
                        if !handle_client_frame(&state, &chat, &gate, &mut sink, conversation_id, user_id, &text).await {
                            break;
                        }
                    }
                    Some(Ok(WsMessage::Close(_))) | None => break,
                    _ => {}
                }
            }
            outgoing = rx.recv() => {
                match outgoing {
                    Ok(frame) => {
                        if sink.send(WsMessage::Text(frame.into())).await.is_err() {
                            break;
                        }
                    }
                    // Overrun: the client re-syncs from REST on demand; never
                    // wedge the loop over a lagged channel.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_))
                    | Err(tokio::sync::broadcast::error::RecvError::Closed) => {}
                }
            }
        }
    }

    // Presence: offline + durable last_seen.
    let _ = chat.set_presence(user_id, "offline").await;
    broadcast_chat(
        &state,
        conversation_id,
        "presence",
        json!({ "user_id": user_id.to_string(), "status": "offline" }),
    );
}

/// Handle one client frame; returns `false` to close the connection.
async fn handle_client_frame(
    state: &AppState,
    chat: &Chat,
    gate: &MessageGate,
    sink: &mut futures::stream::SplitSink<WebSocket, WsMessage>,
    conversation_id: Uuid,
    user_id: Uuid,
    text: &str,
) -> bool {
    let parsed: ClientFrame = match serde_json::from_str(text) {
        Ok(f) => f,
        Err(_) => {
            let _ = sink
                .send(WsMessage::Text(
                    server_frame(
                        "error",
                        conversation_id,
                        json!({ "reason": "malformed frame" }),
                    )
                    .into(),
                ))
                .await;
            return true;
        }
    };
    match parsed {
        ClientFrame::Message { body } => {
            if !gate.allow() {
                let _ = sink
                    .send(WsMessage::Text(
                        server_frame(
                            "error",
                            conversation_id,
                            json!({ "reason": "rate limit exceeded" }),
                        )
                        .into(),
                    ))
                    .await;
                return true;
            }
            match chat.send_message(conversation_id, user_id, &body).await {
                Ok(message) => fan_out_message(state, chat, &message, user_id).await,
                Err(e) => {
                    let _ = sink
                        .send(WsMessage::Text(
                            server_frame(
                                "error",
                                conversation_id,
                                json!({ "reason": e.to_string() }),
                            )
                            .into(),
                        ))
                        .await;
                }
            }
        }
        ClientFrame::Typing => {
            broadcast_chat(
                state,
                conversation_id,
                "typing",
                json!({ "user_id": user_id.to_string() }),
            );
        }
        ClientFrame::Read => {
            if chat.mark_read(conversation_id, user_id).await.is_ok() {
                broadcast_chat(
                    state,
                    conversation_id,
                    "read",
                    json!({ "user_id": user_id.to_string() }),
                );
            }
        }
        ClientFrame::Ack { up_to } => {
            let _ = chat
                .mark_delivered(conversation_id, user_id, up_to.unwrap_or_else(Utc::now))
                .await;
        }
    }
    true
}

/// Fan a persisted message out: chat channel broadcast + per-member
/// notifications (feed + unread). The message row is already committed.
async fn fan_out_message(
    state: &AppState,
    chat: &Chat,
    message: &keystone_db::repositories::chat::Message,
    sender_id: Uuid,
) {
    let conversation_id = message.conversation_id;
    broadcast_chat(
        state,
        conversation_id,
        "message",
        json!({
            "id": message.id.to_string(),
            "sender_id": sender_id.to_string(),
            "body": message.body,
            "sent_at": message.sent_at,
        }),
    );

    // Notify the other members (feed + unread count).
    if let Ok(members) = chat.members(conversation_id).await {
        for member in members {
            if member.user_id == sender_id {
                continue;
            }
            notify(
                &state.pool,
                &state.realtime,
                crate::realtime::Notify {
                    user_id: member.user_id,
                    kind: "message",
                    actor_id: Some(sender_id),
                    entity_type: "conversation",
                    entity_id: Some(conversation_id),
                    payload: json!({
                        "conversation_id": conversation_id.to_string(),
                        "message_id": message.id.to_string(),
                        "preview": message.body.chars().take(120).collect::<String>(),
                    }),
                },
            )
            .await;
        }
    }
}

/// Broadcast a chat event to the conversation's WS subscribers via the hub.
fn broadcast_chat(state: &AppState, conversation_id: Uuid, kind: &str, payload: JsonValue) {
    let frame = server_frame(kind, conversation_id, payload);
    state.realtime.publish_chat(conversation_id, frame);
}
