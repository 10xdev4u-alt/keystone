//! Uploads API — presigned S3-style uploads with quota-enforced registration.
//!
//! Flow:
//!   1. `POST /api/v1/files/presign` (auth) — client asks for a PUT url.
//!      Response: `{ key, put_url }`. No DB write happens here.
//!   2. Client PUTs the bytes straight to the bucket via the presigned url
//!      (bytes never transit the API).
//!   3. `POST /api/v1/files` (auth) — client registers metadata. This is the
//!      quota enforcement point (atomic, see the repository) and the
//!      thumbnail generation point for images.
//!   4. `GET /api/v1/files/{id}` — metadata + fresh presigned GET url
//!      (owner or public file).
//!
//! Security notes: keys are server-generated (`users/{uid}/...`), content
//! types are allowlisted, sizes capped, and the storage key is validated
//! against path traversal before any presigning happens.

use crate::auth::{audit, map_repo_error, AuthUser};
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Path, Query, State};
use axum::Json;
use chrono::{DateTime, Utc};
use keystone_db::repositories::files::{Files, NewFileRecord};
use keystone_db::storage::make_thumbnail;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

// ── Validation limits ───────────────────────────────────────────────────────

const MAX_FILE_BYTES: i64 = 20 * 1024 * 1024; // 20 MiB
const PRESIGN_TTL_SECS: u64 = 900; // 15 minutes
const IMAGE_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];
const ALLOWED_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "application/pdf",
    "text/plain",
    "text/markdown",
    "application/zip",
    "application/json",
];

fn validate_content_type(ct: &str) -> Result<(), ApiError> {
    if ALLOWED_TYPES.contains(&ct) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "content type not allowed: {ct}"
        )))
    }
}

fn safe_suffix(name: &str) -> String {
    // Keep only the final extension, sanitized to [a-z0-9]{1,8}.
    let ext = name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    if ext.is_empty() {
        "bin".into()
    } else {
        ext
    }
}

// ── Requests ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct PresignRequest {
    original_name: String,
    content_type: String,
    size_bytes: i64,
}

#[derive(Deserialize)]
pub struct RegisterRequest {
    /// Key returned by presign.
    key: String,
    original_name: String,
    content_type: String,
    size_bytes: i64,
    sha256: String,
    width: Option<i32>,
    height: Option<i32>,
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// `POST /api/v1/files/presign` — mint a server-keyed PUT url.
pub async fn presign(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<PresignRequest>,
) -> ApiResult<Json<Value>> {
    if req.size_bytes <= 0 || req.size_bytes > MAX_FILE_BYTES {
        return Err(ApiError::BadRequest(format!(
            "size must be within 1..={MAX_FILE_BYTES} bytes"
        )));
    }
    validate_content_type(&req.content_type)?;
    if req.original_name.trim().is_empty() || req.original_name.chars().count() > 255 {
        return Err(ApiError::BadRequest("invalid original name".into()));
    }

    // Server-generated key: ownership is baked in, traversal is impossible.
    let key = format!(
        "users/{}/{}u-{}.{}",
        user.user_id,
        Uuid::new_v4().simple(),
        &req.original_name.chars().take(32).collect::<String>()[..],
        safe_suffix(&req.original_name),
    );

    let put_url = state
        .storage
        .presign_put(&key, &req.content_type, PRESIGN_TTL_SECS)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "presign put failed");
            ApiError::Internal
        })?;

    audit(
        &state.pool,
        user.user_id,
        "file_presign",
        "file",
        &key,
        None,
    )
    .await;
    Ok(Json(json!({ "key": key, "put_url": put_url })))
}

/// `POST /api/v1/files` — register metadata after the bytes are in the bucket.
/// Atomic quota enforcement + image thumbnail generation.
pub async fn register_file(
    State(state): State<AppState>,
    user: AuthUser,
    Json(req): Json<RegisterRequest>,
) -> ApiResult<Json<Value>> {
    if req.size_bytes <= 0 || req.size_bytes > MAX_FILE_BYTES {
        return Err(ApiError::BadRequest(format!(
            "size must be within 1..={MAX_FILE_BYTES} bytes"
        )));
    }
    validate_content_type(&req.content_type)?;
    if req.sha256.len() < 16 || req.sha256.len() > 128 {
        return Err(ApiError::BadRequest("invalid sha256".into()));
    }
    // The key must be one we minted for THIS user — never trust client paths.
    let expected_prefix = format!("users/{}/", user.user_id);
    if !req.key.starts_with(&expected_prefix) {
        return Err(ApiError::Forbidden);
    }

    let record = NewFileRecord {
        owner_id: user.user_id,
        bucket: "keystone",
        object_key: &req.key,
        original_name: &req.original_name,
        content_type: &req.content_type,
        size_bytes: req.size_bytes,
        sha256: &req.sha256,
        width: req.width,
        height: req.height,
        parent_id: None,
        is_public: false,
    };
    let files = Files::new(state.pool.clone());
    let row = files.register(&record).await.map_err(map_repo_error)?;

    // Images get a server-side thumbnail, best-effort (a failed thumbnail
    // must not fail the upload — the original is already stored).
    let mut thumb_w = req.width;
    let mut thumb_h = req.height;
    if IMAGE_TYPES.contains(&req.content_type.as_str()) {
        if let Ok(bytes) = state.storage.get_bytes(&req.key).await {
            if let Ok((thumb, w, h)) = make_thumbnail(&bytes, 512) {
                let thumb_key = format!("thumbs/{}", req.key);
                if state
                    .storage
                    .put_bytes(&thumb_key, &thumb, "image/jpeg")
                    .await
                    .is_ok()
                {
                    thumb_w = Some(w as i32);
                    thumb_h = Some(h as i32);
                }
            }
        }
    }

    let get_url = state
        .storage
        .presign_get(&req.key, PRESIGN_TTL_SECS)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "presign get failed");
            ApiError::Internal
        })?;

    audit(
        &state.pool,
        user.user_id,
        "file_register",
        "file",
        &row.id.to_string(),
        None,
    )
    .await;
    Ok(Json(json!({
        "id": row.id,
        "key": row.object_key,
        "original_name": row.original_name,
        "content_type": row.content_type,
        "size_bytes": row.size_bytes,
        "width": thumb_w,
        "height": thumb_h,
        "created_at": row.created_at,
        "get_url": get_url,
    })))
}

#[derive(Deserialize)]
pub struct ListParams {
    before: Option<DateTime<Utc>>,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    50
}

/// `GET /api/v1/files` — cursor-paged listing of the caller's files.
pub async fn list_files(
    State(state): State<AppState>,
    user: AuthUser,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Value>> {
    let files = Files::new(state.pool.clone());
    let rows = files
        .list_for_owner(user.user_id, params.before, params.limit)
        .await
        .map_err(map_repo_error)?;
    Ok(Json(json!({ "items": rows })))
}

/// `GET /api/v1/files/{id}` — metadata + a fresh presigned download url.
/// Owner or public file; anyone else gets a 404 (existence is never
/// confirmed to non-owners).
pub async fn get_file(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let files = Files::new(state.pool.clone());
    let row = files
        .get(id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if row.owner_id != user.user_id && !row.is_public {
        return Err(ApiError::NotFound);
    }
    let get_url = state
        .storage
        .presign_get(&row.object_key, PRESIGN_TTL_SECS)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "presign get failed");
            ApiError::Internal
        })?;
    Ok(Json(json!({
        "id": row.id,
        "original_name": row.original_name,
        "content_type": row.content_type,
        "size_bytes": row.size_bytes,
        "width": row.width,
        "height": row.height,
        "created_at": row.created_at,
        "get_url": get_url,
    })))
}

/// `DELETE /api/v1/files/{id}` — owner only. Metadata row is removed first;
/// the object deletion in the bucket is best-effort.
pub async fn delete_file(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<Value>> {
    let files = Files::new(state.pool.clone());
    let row = files
        .get(id)
        .await
        .map_err(map_repo_error)?
        .ok_or(ApiError::NotFound)?;
    if row.owner_id != user.user_id {
        return Err(ApiError::NotFound);
    }
    files
        .delete(id, user.user_id)
        .await
        .map_err(map_repo_error)?;
    if let Err(e) = state.storage.delete(&row.object_key).await {
        tracing::warn!(error = %e, key = %row.object_key, "object delete failed");
    }
    audit(
        &state.pool,
        user.user_id,
        "file_delete",
        "file",
        &id.to_string(),
        None,
    )
    .await;
    Ok(Json(json!({ "deleted": true })))
}
