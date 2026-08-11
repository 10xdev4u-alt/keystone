//! Unified search — the one place the platform is searched.
//!
//! Month 7 built the Postgres FTS engine (`keystone_db::search`) with
//! typo-tolerant `pg_trgm` fallback and weighted `ts_rank`; this route is the
//! missing public surface for it. The `SearchBackend` trait keeps an external
//! engine (Elasticsearch) swappable behind the same handler.

use crate::auth::map_repo_error;
use crate::error::{ApiError, ApiResult};
use crate::AppState;
use axum::extract::{Query, State};
use axum::Json;
use keystone_db::search::SearchBackend;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// One ranked hit across posts, users, communities and courses.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchHitView {
    pub entity_type: String,
    pub entity_id: String,
    pub title: String,
    pub snippet: String,
    pub score: f64,
}

/// Search response — ranked hits for a query.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchResponse {
    pub query: String,
    pub results: Vec<SearchHitView>,
}

/// Search request.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SearchQuery {
    /// The raw search string (FTS lexemes + typo fallback).
    pub q: String,
    /// Page size (1..=50).
    #[serde(default)]
    pub limit: Option<i64>,
}

/// Search everything the engine indexes.
#[utoipa::path(
    get,
    path = "/api/v1/search",
    params(
        ("q" = String, Query, description = "Search query"),
        ("limit" = Option<i64>, Query, description = "Page size (1..=50)"),
    ),
    responses(
        (status = 200, description = "Ranked hits", body = SearchResponse),
        (status = 400, description = "Empty query"),
    ),
    tag = "search"
)]
pub async fn search(
    State(state): State<AppState>,
    Query(query): Query<SearchQuery>,
) -> ApiResult<Json<SearchResponse>> {
    let q = query.q.trim();
    if q.is_empty() {
        return Err(ApiError::BadRequest(
            "search query must not be empty".into(),
        ));
    }
    let backend = keystone_db::search::PostgresSearch::new(state.pool.clone());
    let hits = backend
        .search(q, query.limit.unwrap_or(20).clamp(1, 50) as usize)
        .await
        .map_err(map_repo_error)?;
    let results = hits
        .into_iter()
        .map(|hit| SearchHitView {
            entity_type: hit.entity_type,
            entity_id: hit.entity_id.to_string(),
            title: hit.title,
            snippet: hit.snippet,
            score: hit.score,
        })
        .collect();
    Ok(Json(SearchResponse {
        query: q.to_string(),
        results,
    }))
}
