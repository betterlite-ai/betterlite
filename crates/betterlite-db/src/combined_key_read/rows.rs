use std::collections::BTreeMap;
use std::sync::PoisonError;

use chrono::{DateTime, Utc};

use crate::auth_reconnect::request::ReconnectRequest;
use crate::auth_reconnect::stale::StaleReadEngine;
use crate::budget_permission_rows::JsonData;
use crate::database_availability::classification::STALE_PLAN_MARKER;
use crate::database_failures::metrics::DbServiceEventSink;
use crate::database_faults::DbError;
use crate::key_hash::KeyHash;
use crate::key_user_writes::retry::WriteClock;

use super::error::KeyReadError;
use super::grace::DeprecatedToken;
use super::ports::{Diagnostic, KeyReadDatabase, KeyReadDiagnostics, PlanReconnect};
use super::reader::KeyReader;
use super::view::ViewError;

pub(crate) const RECONNECT_REASON: &str = "postgres_cached_plan_error";
pub(crate) const CACHED_PLAN_WARNING: &str = "PostgreSQL cached plan error detected for token lookup; recreating the database connection and retrying with the same query. This may occur during rolling deployments when schema changes are applied.";

impl<Database, Reconnect, Sink> KeyReader<'_, Database, Reconnect, Sink>
where
    Database: KeyReadDatabase,
    Reconnect: PlanReconnect,
    Sink: DbServiceEventSink + 'static,
{
    // a stale plan on a replica read reconnects once and retries the same query, whatever the reconnect reports
    pub(super) async fn read_row(
        &self,
        hash: &KeyHash,
        use_writer: bool,
    ) -> Result<Option<BTreeMap<String, JsonData>>, KeyReadError> {
        if use_writer {
            return self.row_query(hash, true).await;
        }
        let observed = self.reconnect.read_target();
        match self.database.read_row(hash, false).await {
            Ok(row) => row_fields(row),
            Err(error) if is_cached_plan(&error) => {
                self.services.diagnostics.emit(Diagnostic::CachedPlan {
                    message: CACHED_PLAN_WARNING,
                });
                let request = reconnect_request();
                let stale = StaleReadEngine::observe(&observed);
                self.reconnect.attempt(&request, stale.as_ref()).await?;
                self.row_query(hash, false).await
            }
            Err(error) => Err(KeyReadError::Database(error)),
        }
    }

    async fn row_query(
        &self,
        hash: &KeyHash,
        use_writer: bool,
    ) -> Result<Option<BTreeMap<String, JsonData>>, KeyReadError> {
        let row = self
            .database
            .read_row(hash, use_writer)
            .await
            .map_err(KeyReadError::Database)?;
        row_fields(row)
    }

    // a writer read still fills the grace cache, but it never reads a value another request cached
    pub(super) async fn grace_lookup(&self, hash: &KeyHash, use_writer: bool) -> Option<String> {
        let now = self.services.clock.now();
        if !use_writer && let Some(cached) = self.cached_grace(hash, now) {
            return Some(cached);
        }
        match self.database.deprecated_row(hash, now, use_writer).await {
            Ok(Some(deprecated)) => self.remember_grace(hash, &deprecated, now),
            Ok(None) => None,
            Err(error) => {
                let message = format!("Deprecated key lookup skipped: {error}");
                self.services
                    .diagnostics
                    .emit(Diagnostic::DeprecatedLookupFailed { message: &message });
                None
            }
        }
    }

    fn cached_grace(&self, hash: &KeyHash, now: DateTime<Utc>) -> Option<String> {
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.lookup(hash, now)
    }

    fn remember_grace(
        &self,
        hash: &KeyHash,
        deprecated: &DeprecatedToken,
        now: DateTime<Utc>,
    ) -> Option<String> {
        if deprecated.active_token_id.is_empty() {
            return None;
        }
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        cache.remember(hash.clone(), deprecated, now);
        Some(deprecated.active_token_id.clone())
    }
}

fn row_fields(row: Option<JsonData>) -> Result<Option<BTreeMap<String, JsonData>>, KeyReadError> {
    match row {
        None => Ok(None),
        Some(JsonData::Object(fields)) => Ok(Some(fields)),
        Some(_other) => Err(KeyReadError::View(ViewError::NotAnObject)),
    }
}

fn is_cached_plan(error: &DbError) -> bool {
    // LiteLLM matches the marker as written, case-sensitively, unlike the availability classifier
    error.message.contains(STALE_PLAN_MARKER)
}

fn reconnect_request<'request>() -> ReconnectRequest<'request> {
    ReconnectRequest {
        reason: RECONNECT_REASON,
        force: false,
        force_recreate: true,
        timeout: None,
        lock_timeout: None,
        database_url: None,
    }
}
