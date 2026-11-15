use std::sync::Arc;

use async_trait::async_trait;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

use crate::tracing_routes::access::{TraceAuth, TraceRole};
use crate::tracing_routes::receiver::TraceRuntime;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum QueryScope {
    Admin,
    Team {
        team_id: String,
    },
    Key {
        team_id: String,
        api_key_hash: String,
    },
}

#[derive(Debug, Error)]
pub enum QueryAccessError {
    #[error("Trace SQL queries require a configured proxy master key")]
    SecretMissing,
    #[error("Project trace SQL queries require a project key")]
    ProjectKeyRequired,
    #[error("Trace SQL queries require an authenticated trace scope")]
    ScopeRequired,
    #[error("Agent tracing is not enabled. Set `tracing:` in general_settings and CLICKHOUSE_URL.")]
    Unavailable,
}

#[derive(Debug, Error)]
pub enum QueryRunError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Unavailable(String),
}

#[async_trait]
pub trait TraceQueryStore: Send + Sync {
    async fn query_sql(
        &self,
        sql: &str,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError>;
    async fn query_help(
        &self,
        scope: &QueryScope,
        secret: &SecretString,
    ) -> Result<String, QueryRunError>;
}

pub struct TraceQueryAccess {
    pub store: Arc<dyn TraceQueryStore>,
    pub scope: QueryScope,
    pub secret: SecretString,
}

pub fn trace_query_scope(auth: &TraceAuth) -> Result<QueryScope, QueryAccessError> {
    if matches!(auth.role, TraceRole::Admin | TraceRole::AdminViewOnly) {
        return Ok(QueryScope::Admin);
    }
    if auth.project_id.is_some() {
        let Some(token) = auth.token.as_ref() else {
            return Err(QueryAccessError::ProjectKeyRequired);
        };
        return Ok(QueryScope::Key {
            team_id: auth.team_id.clone().unwrap_or_default(),
            api_key_hash: token.clone(),
        });
    }
    if let Some(team_id) = auth.team_id.as_ref() {
        return Ok(QueryScope::Team {
            team_id: team_id.clone(),
        });
    }
    auth.token
        .as_ref()
        .map_or(Err(QueryAccessError::ScopeRequired), |token| {
            Ok(QueryScope::Key {
                team_id: String::new(),
                api_key_hash: token.clone(),
            })
        })
}

pub fn provide_trace_query_secret(
    master_key: Option<&SecretString>,
) -> Result<SecretString, QueryAccessError> {
    match master_key {
        Some(key) if !key.expose_secret().is_empty() => Ok(key.clone()),
        Some(_) | None => Err(QueryAccessError::SecretMissing),
    }
}

pub fn provide_trace_query_access(
    auth: &TraceAuth,
    receiver: Option<&Arc<dyn TraceRuntime>>,
    master_key: Option<&SecretString>,
) -> Result<TraceQueryAccess, QueryAccessError> {
    let secret = provide_trace_query_secret(master_key)?;
    let found = receiver.ok_or(QueryAccessError::Unavailable)?;
    let scope = trace_query_scope(auth)?;
    Ok(TraceQueryAccess {
        store: found.query_store(),
        scope,
        secret,
    })
}
