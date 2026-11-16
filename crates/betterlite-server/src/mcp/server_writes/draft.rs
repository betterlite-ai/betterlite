use betterlite_db::managed_resource_rows::mcp::McpServerTable;
use betterlite_db::mcp::table_access::rows::server_id_of;
use betterlite_db::mcp_server_encoding::request::McpServerRequest;
use betterlite_db::stored_cipher::DbCipher;
use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use super::create::create_mcp_server;
use super::decode::{decode_server_row, row_as};
use super::delete::delete_mcp_server;
use super::{DRAFT, DraftOptions, ServerWriteError, ServerWriteStore, TokenCacheInvalidation};

#[derive(Deserialize)]
struct TimestampRow {
    #[serde(default)]
    updated_at: Option<String>,
}

#[derive(Deserialize)]
struct StatusRow {
    #[serde(default)]
    approval_status: Option<String>,
}

// LiteLLM's create_draft_mcp_server: prune, create-if-absent, adopt the winner of a create race.
pub async fn create_draft_mcp_server<S, I>(
    store: &S,
    cipher: &DbCipher,
    request: &McpServerRequest,
    options: &DraftOptions,
    invalidation: &I,
) -> Result<McpServerTable, ServerWriteError>
where
    S: ServerWriteStore,
    I: TokenCacheInvalidation,
{
    let draft_id = options
        .server_id
        .clone()
        .or_else(|| request.server_id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    prune_expired_drafts(store, options.ttl_seconds, invalidation).await?;
    // An existing row, a live draft or the real server the edit form re-authorizes, is never overwritten.
    if let Some(existing) = store.find_server(&draft_id).await? {
        return decode_server_row(existing, cipher);
    }
    let draft = draft_request(request, draft_id.clone());
    match create_mcp_server(store, cipher, &draft, &options.touched_by).await {
        Ok(created) => Ok(created),
        Err(error) => adopt_raced_draft(store, cipher, &draft_id, error).await,
    }
}

// LiteLLM's get_draft_mcp_server: the TTL is part of the query, so an expired draft is unreadable at once.
pub async fn get_draft_mcp_server<S>(
    store: &S,
    cipher: &DbCipher,
    server_id: &str,
    ttl_seconds: i64,
) -> Result<Option<McpServerTable>, ServerWriteError>
where
    S: ServerWriteStore,
{
    let cutoff = draft_cutoff(ttl_seconds);
    let Some(row) = store.find_draft_updated_since(server_id, cutoff).await? else {
        return Ok(None);
    };
    decode_server_row(row, cipher).map(Some)
}

// LiteLLM's _prune_expired_draft_mcp_servers: runs on every draft write, bounded by admin authorizations.
pub async fn prune_expired_drafts<S, I>(
    store: &S,
    ttl_seconds: i64,
    invalidation: &I,
) -> Result<usize, ServerWriteError>
where
    S: ServerWriteStore,
    I: TokenCacheInvalidation,
{
    let cutoff = draft_cutoff(ttl_seconds);
    let mut pruned = 0;
    for row in store.find_drafts().await? {
        let stored: TimestampRow = row_as(&row)?;
        let Some(updated_at) = stored.updated_at.as_deref().and_then(stored_utc) else {
            // A row without a timestamp has no age to judge, so it is left rather than guessed stale.
            continue;
        };
        if updated_at >= cutoff {
            continue;
        }
        let server_id = server_id_of(&row)?;
        // Another worker sweeping the same row is harmless: a row already gone is a no-op.
        if delete_mcp_server(store, &server_id, invalidation)
            .await?
            .is_some()
        {
            pruned = pruned.saturating_add(1);
        }
    }
    Ok(pruned)
}

async fn adopt_raced_draft<S>(
    store: &S,
    cipher: &DbCipher,
    draft_id: &str,
    error: ServerWriteError,
) -> Result<McpServerTable, ServerWriteError>
where
    S: ServerWriteStore,
{
    // The loser of the read-then-create race adopts the winner's draft; anything else keeps its error.
    let Some(row) = store.find_server(draft_id).await? else {
        return Err(error);
    };
    let status: StatusRow = row_as(&row)?;
    if status.approval_status.as_deref() != Some(DRAFT) {
        return Err(error);
    }
    decode_server_row(row, cipher)
}

fn draft_request(request: &McpServerRequest, draft_id: String) -> McpServerRequest {
    let mut draft = request.clone();
    draft.server_id = Some(draft_id);
    draft.approval_status = Some(DRAFT.to_owned());
    draft
}

// A TTL too large to subtract from now keeps every draft, the oldest instant being the cutoff.
fn draft_cutoff(ttl_seconds: i64) -> DateTime<Utc> {
    Duration::try_seconds(ttl_seconds.max(1))
        .and_then(|window| Utc::now().checked_sub_signed(window))
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

fn stored_utc(text: &str) -> Option<DateTime<Utc>> {
    if let Ok(aware) = DateTime::parse_from_rfc3339(text) {
        return Some(aware.with_timezone(&Utc));
    }
    for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f"] {
        if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
            return Some(naive.and_utc());
        }
    }
    None
}
