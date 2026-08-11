//! Chat repository — conversations, memberships, messages, presence.
//!
//! Authorization primitive: membership. Every message read/write and every
//! presence read is gated on `conversation_members` — never on the
//! conversation id alone. Direct conversations are unique per unordered pair
//! (`conversations_direct_pair_key`), and the find-or-create insert is
//! race-safe via `ON CONFLICT DO NOTHING` + re-select.
//!
//! Messages persist through this repository (the "normal API write path");
//! WebSockets are a thin transport on top — delivery acks and presence, not
//! the source of truth.

use super::RepoError;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Conversation {
    pub id: Uuid,
    pub kind: String,
    pub title: Option<String>,
    pub user_a: Option<Uuid>,
    pub user_b: Option<Uuid>,
    pub created_by: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub last_message_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConversationWithLast {
    pub id: Uuid,
    pub kind: String,
    pub title: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_message_at: DateTime<Utc>,
    pub last_message: Option<String>,
    pub last_message_at2: Option<DateTime<Utc>>,
    pub unread: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Message {
    pub id: Uuid,
    pub conversation_id: Uuid,
    pub sender_id: Uuid,
    pub body: String,
    pub sent_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub read_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ConversationMember {
    pub conversation_id: Uuid,
    pub user_id: Uuid,
    pub joined_at: DateTime<Utc>,
    pub last_read_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Presence {
    pub user_id: Uuid,
    pub status: String,
    pub last_seen_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct Chat {
    pool: PgPool,
}

impl Chat {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Find or create a direct conversation between two users (race-safe).
    pub async fn find_or_create_direct(
        &self,
        user_a: Uuid,
        user_b: Uuid,
    ) -> Result<Conversation, RepoError> {
        if user_a == user_b {
            return Err(RepoError::InvalidInput("cannot message yourself".into()));
        }
        // Order the pair so the unique index is hit consistently.
        let (a, b) = if user_a < user_b {
            (user_a, user_b)
        } else {
            (user_b, user_a)
        };
        let inserted = sqlx::query_as::<_, Conversation>(
            r#"
            INSERT INTO conversations (kind, user_a, user_b, created_by)
            VALUES ('direct', $1, $2, $1)
            ON CONFLICT DO NOTHING
            RETURNING id, kind, title, user_a, user_b, created_by, created_at, last_message_at
            "#,
        )
        .bind(a)
        .bind(b)
        .fetch_optional(&self.pool)
        .await?;

        let conversation = match inserted {
            Some(c) => {
                // First membership insertion wins; the loser is already a member.
                for member in [a, b] {
                    sqlx::query(
                        "INSERT INTO conversation_members (conversation_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                    )
                    .bind(c.id)
                    .bind(member)
                    .execute(&self.pool)
                    .await?;
                }
                c
            }
            None => {
                sqlx::query_as::<_, Conversation>(
                    r#"
                SELECT id, kind, title, user_a, user_b, created_by, created_at, last_message_at
                FROM conversations
                WHERE kind = 'direct' AND user_a = $1 AND user_b = $2
                "#,
                )
                .bind(a)
                .bind(b)
                .fetch_one(&self.pool)
                .await?
            }
        };
        Ok(conversation)
    }

    /// Create a group conversation; creator becomes the first member.
    pub async fn create_group(
        &self,
        created_by: Uuid,
        title: &str,
        member_ids: &[Uuid],
    ) -> Result<Conversation, RepoError> {
        let title = title.trim();
        if title.is_empty() {
            return Err(RepoError::InvalidInput("title cannot be empty".into()));
        }
        if title.chars().count() > 120 {
            return Err(RepoError::InvalidInput("title too long".into()));
        }
        let mut tx = self.pool.begin().await?;
        let conversation = sqlx::query_as::<_, Conversation>(
            r#"
            INSERT INTO conversations (kind, title, created_by)
            VALUES ('group', $1, $2)
            RETURNING id, kind, title, user_a, user_b, created_by, created_at, last_message_at
            "#,
        )
        .bind(title)
        .bind(created_by)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO conversation_members (conversation_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(conversation.id)
        .bind(created_by)
        .execute(&mut *tx)
        .await?;
        for member in member_ids {
            sqlx::query(
                "INSERT INTO conversation_members (conversation_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(conversation.id)
            .bind(member)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(conversation)
    }

    pub async fn is_member(&self, conversation_id: Uuid, user_id: Uuid) -> Result<bool, RepoError> {
        // `fetch_optional`, never `fetch_one` — a non-member has NO row, and
        // `fetch_one` would surface a RowNotFound as a 500 instead of a clean
        // "not a member" answer.
        let member = sqlx::query_scalar::<_, Uuid>(
            "SELECT user_id FROM conversation_members WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(conversation_id)
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(member.is_some())
    }

    pub async fn add_member(
        &self,
        conversation_id: Uuid,
        actor: Uuid,
        user_id: Uuid,
    ) -> Result<(), RepoError> {
        // Group membership changes are actor-gated: only existing members add.
        if !self.is_member(conversation_id, actor).await? {
            return Err(RepoError::InvalidInput("not a member".into()));
        }
        sqlx::query(
            "INSERT INTO conversation_members (conversation_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(conversation_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_member(
        &self,
        conversation_id: Uuid,
        actor: Uuid,
        user_id: Uuid,
    ) -> Result<(), RepoError> {
        if !self.is_member(conversation_id, actor).await? {
            return Err(RepoError::InvalidInput("not a member".into()));
        }
        sqlx::query("DELETE FROM conversation_members WHERE conversation_id = $1 AND user_id = $2")
            .bind(conversation_id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// All conversations for a user, with last message + unread count.
    pub async fn list_for_user(
        &self,
        user_id: Uuid,
    ) -> Result<Vec<ConversationWithLast>, RepoError> {
        sqlx::query_as::<_, ConversationWithLast>(
            r#"
            SELECT c.id, c.kind, c.title, c.created_at, c.last_message_at,
                   (SELECT m.body FROM messages m
                    WHERE m.conversation_id = c.id
                    ORDER BY m.sent_at DESC LIMIT 1) AS last_message,
                   (SELECT m.sent_at FROM messages m
                    WHERE m.conversation_id = c.id
                    ORDER BY m.sent_at DESC LIMIT 1) AS last_message_at2,
                   (SELECT count(*) FROM messages m
                    WHERE m.conversation_id = c.id
                      AND m.sender_id <> $1
                      AND m.sent_at > cm.last_read_at) AS unread
            FROM conversations c
            JOIN conversation_members cm ON cm.conversation_id = c.id
            WHERE cm.user_id = $1
            ORDER BY c.last_message_at DESC
            "#,
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Members of a conversation (callers must check membership first).
    pub async fn members(
        &self,
        conversation_id: Uuid,
    ) -> Result<Vec<ConversationMember>, RepoError> {
        sqlx::query_as::<_, ConversationMember>(
            r#"
            SELECT conversation_id, user_id, joined_at, last_read_at
            FROM conversation_members
            WHERE conversation_id = $1
            ORDER BY joined_at
            "#,
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Cursor-paged messages, newest-first (chat history pages back in time
    /// through the `before` cursor).
    pub async fn list_messages(
        &self,
        conversation_id: Uuid,
        user_id: Uuid,
        before: Option<DateTime<Utc>>,
        limit: i64,
    ) -> Result<Vec<Message>, RepoError> {
        if !self.is_member(conversation_id, user_id).await? {
            return Err(RepoError::InvalidInput("not a member".into()));
        }
        let limit = limit.clamp(1, 200);
        sqlx::query_as::<_, Message>(
            r#"
            SELECT id, conversation_id, sender_id, body, sent_at, edited_at, delivered_at, read_at
            FROM messages
            WHERE conversation_id = $1
              AND ($2::timestamptz IS NULL OR sent_at < $2)
            ORDER BY sent_at DESC
            LIMIT $3
            "#,
        )
        .bind(conversation_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }

    /// Persist a message; sender must be a member. Updates `last_message_at`.
    pub async fn send_message(
        &self,
        conversation_id: Uuid,
        sender_id: Uuid,
        body: &str,
    ) -> Result<Message, RepoError> {
        let body = body.trim();
        if body.is_empty() {
            return Err(RepoError::InvalidInput("message cannot be empty".into()));
        }
        if body.chars().count() > 4000 {
            return Err(RepoError::InvalidInput("message too long".into()));
        }
        if !self.is_member(conversation_id, sender_id).await? {
            return Err(RepoError::InvalidInput("not a member".into()));
        }
        let mut tx = self.pool.begin().await?;
        let message = sqlx::query_as::<_, Message>(
            r#"
            INSERT INTO messages (conversation_id, sender_id, body)
            VALUES ($1, $2, $3)
            RETURNING id, conversation_id, sender_id, body, sent_at, edited_at, delivered_at, read_at
            "#,
        )
        .bind(conversation_id)
        .bind(sender_id)
        .bind(body)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query("UPDATE conversations SET last_message_at = now() WHERE id = $1")
            .bind(conversation_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(message)
    }

    /// Mark messages delivered to `user_id` up to `up_to` (delivery ack).
    pub async fn mark_delivered(
        &self,
        conversation_id: Uuid,
        user_id: Uuid,
        up_to: DateTime<Utc>,
    ) -> Result<u64, RepoError> {
        if !self.is_member(conversation_id, user_id).await? {
            return Err(RepoError::InvalidInput("not a member".into()));
        }
        let result = sqlx::query(
            r#"
            UPDATE messages
            SET delivered_at = COALESCE(delivered_at, now())
            WHERE conversation_id = $1 AND sender_id <> $2 AND sent_at <= $3
            "#,
        )
        .bind(conversation_id)
        .bind(user_id)
        .bind(up_to)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected())
    }

    /// Mark everything up to now as read: bumps the member cursor AND stamps
    /// `read_at` on the messages themselves (single statement each).
    pub async fn mark_read(&self, conversation_id: Uuid, user_id: Uuid) -> Result<(), RepoError> {
        if !self.is_member(conversation_id, user_id).await? {
            return Err(RepoError::InvalidInput("not a member".into()));
        }
        sqlx::query(
            "UPDATE conversation_members SET last_read_at = now() WHERE conversation_id = $1 AND user_id = $2",
        )
        .bind(conversation_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "UPDATE messages SET read_at = COALESCE(read_at, now()) WHERE conversation_id = $1 AND sender_id <> $2 AND sent_at <= now()",
        )
        .bind(conversation_id)
        .bind(user_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Presence ────────────────────────────────────────────────────────────

    pub async fn set_presence(&self, user_id: Uuid, status: &str) -> Result<(), RepoError> {
        sqlx::query(
            r#"
            INSERT INTO presence (user_id, status, last_seen_at)
            VALUES ($1, $2, now())
            ON CONFLICT (user_id)
            DO UPDATE SET status = $2, last_seen_at = now()
            "#,
        )
        .bind(user_id)
        .bind(status)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Presence for a conversation's members — privacy enforced here: the
    /// caller must already be a member (checked by the caller/API layer).
    pub async fn presence_for(&self, conversation_id: Uuid) -> Result<Vec<Presence>, RepoError> {
        sqlx::query_as::<_, Presence>(
            r#"
            SELECT p.user_id, p.status, p.last_seen_at
            FROM presence p
            JOIN conversation_members cm ON cm.user_id = p.user_id
            WHERE cm.conversation_id = $1
            ORDER BY p.user_id
            "#,
        )
        .bind(conversation_id)
        .fetch_all(&self.pool)
        .await
        .map_err(Into::into)
    }
}
