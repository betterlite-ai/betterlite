#![cfg(test)]

use std::future::Future;
use std::sync::{Arc, Mutex};

use betterlite_auth::auth_invalidation::error::InvalidationError;
use betterlite_db::postgres_store::errors::DbError;
use rust_decimal::Decimal;

use super::cache::AllowlistCache;
use super::postgres::PostgresAllowlists;
use super::statement::{AllowlistKind, AllowlistRewrite, TouchedRow};
use super::{AllowlistDatabase, ModelAllowlistError};

// The order the operation must keep: the statement completes before any cache call is made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AllowlistEvent {
    SqlCompleted,
    CacheCall(Vec<String>),
}

#[derive(Clone, Default)]
pub(crate) struct EventLog {
    events: Arc<Mutex<Vec<AllowlistEvent>>>,
}

impl EventLog {
    pub(crate) fn record(&self, event: AllowlistEvent) {
        let mut events = self.events.lock().expect("events");
        events.push(event);
    }

    pub(crate) fn recorded(&self) -> Vec<AllowlistEvent> {
        let events = self.events.lock().expect("events");
        events.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScriptedRow {
    pub(crate) kind: AllowlistKind,
    pub(crate) object_id: String,
    pub(crate) team_alias: Option<String>,
    // None is the NULL array LiteLLM's columns allow; Some(Vec::new()) is the empty one.
    pub(crate) models: Option<Vec<String>>,
}

pub(crate) fn row(
    kind: AllowlistKind,
    object_id: &str,
    team_alias: Option<&str>,
    models: Option<&[&str]>,
) -> ScriptedRow {
    let stored = models.map(|names| names.iter().map(|name| (*name).to_owned()).collect());
    ScriptedRow {
        kind,
        object_id: object_id.to_owned(),
        team_alias: team_alias.map(str::to_owned),
        models: stored,
    }
}

// The array statements over rows whose allowlists keep duplicates and order.
pub(crate) struct ScriptedAllowlists {
    pub(crate) rows: Mutex<Vec<ScriptedRow>>,
    pub(crate) calls: Mutex<Vec<AllowlistRewrite>>,
    pub(crate) events: EventLog,
    pub(crate) failure: bool,
    pub(crate) reverse_rows: bool,
}

impl ScriptedAllowlists {
    pub(crate) fn new(rows: Vec<ScriptedRow>, events: EventLog) -> Self {
        Self {
            rows: Mutex::new(rows),
            calls: Mutex::new(Vec::new()),
            events,
            failure: false,
            reverse_rows: false,
        }
    }

    fn apply(&self, rewrite: &AllowlistRewrite) -> Vec<TouchedRow> {
        let mut rows = self.rows.lock().expect("rows");
        let mut touched = Vec::new();
        for row in rows.iter_mut() {
            let Some(models) = row.models.as_mut() else {
                continue;
            };
            let applied = match rewrite {
                AllowlistRewrite::Append { old_name, new_name } => {
                    append_name(models, old_name, new_name)
                }
                AllowlistRewrite::Replace { old_name, new_name } => {
                    replace_name(models, old_name, new_name)
                }
            };
            if applied {
                touched.push(TouchedRow {
                    kind: row.kind,
                    object_id: row.object_id.clone(),
                    team_alias: row.team_alias.clone(),
                });
            }
        }
        drop(rows);
        if self.reverse_rows {
            touched.reverse();
        }
        touched
    }
}

fn append_name(models: &mut Vec<String>, old_name: &str, new_name: &str) -> bool {
    let holds_old = models.iter().any(|name| name == old_name);
    let holds_new = models.iter().any(|name| name == new_name);
    if !holds_old || holds_new {
        return false;
    }
    models.push(new_name.to_owned());
    true
}

fn replace_name(models: &mut Vec<String>, old_name: &str, new_name: &str) -> bool {
    if !models.iter().any(|name| name == old_name) {
        return false;
    }
    let replaced = models
        .iter()
        .filter(|name| name.as_str() != new_name)
        .map(|name| {
            if name == old_name {
                new_name.to_owned()
            } else {
                name.clone()
            }
        })
        .collect();
    *models = replaced;
    true
}

impl AllowlistDatabase for ScriptedAllowlists {
    fn rewrite_allowlists(
        &self,
        rewrite: &AllowlistRewrite,
    ) -> impl Future<Output = Result<Vec<TouchedRow>, ModelAllowlistError>> + Send {
        self.calls.lock().expect("calls").push(rewrite.clone());
        let outcome = if self.failure {
            Err(ModelAllowlistError::Database(DbError::NoDbConnected))
        } else {
            let touched = self.apply(rewrite);
            self.events.record(AllowlistEvent::SqlCompleted);
            Ok(touched)
        };
        async move { outcome }
    }
}

pub(crate) struct ScriptedAllowlistCache {
    pub(crate) calls: Mutex<Vec<Vec<String>>>,
    pub(crate) events: EventLog,
    pub(crate) failure: Option<i64>,
}

impl ScriptedAllowlistCache {
    pub(crate) fn recording(events: EventLog) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            events,
            failure: None,
        }
    }

    pub(crate) fn failing(events: EventLog, code: i64) -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
            events,
            failure: Some(code),
        }
    }
}

impl AllowlistCache for ScriptedAllowlistCache {
    fn evict_and_broadcast(
        &self,
        cache_keys: &[String],
    ) -> impl Future<Output = Result<(), InvalidationError>> + Send {
        let keys = cache_keys.to_vec();
        self.calls.lock().expect("calls").push(keys.clone());
        self.events.record(AllowlistEvent::CacheCall(keys));
        let outcome = self.failure.map_or(Ok(()), |code| {
            Err(InvalidationError::SpendOutOfRange(Decimal::new(code, 0)))
        });
        async move { outcome }
    }
}

// Delegates to the real statement so the event log records the SQL completing, not a substitute for it.
pub(crate) struct ObservedAllowlists<'a> {
    pub(crate) inner: PostgresAllowlists<'a>,
    pub(crate) events: EventLog,
}

impl AllowlistDatabase for ObservedAllowlists<'_> {
    async fn rewrite_allowlists(
        &self,
        rewrite: &AllowlistRewrite,
    ) -> Result<Vec<TouchedRow>, ModelAllowlistError> {
        let touched = self.inner.rewrite_allowlists(rewrite).await?;
        self.events.record(AllowlistEvent::SqlCompleted);
        Ok(touched)
    }
}
