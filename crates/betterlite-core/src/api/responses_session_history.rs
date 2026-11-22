use std::error::Error;
use std::future::Future;

use betterlite_config::api::responses_session_history::SessionLookupSettings;
use betterlite_context::local_ttl_cache::dual::DualLocalCache;
use betterlite_db::auxiliary_repository::repository::TableError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::api::api_boundaries::ApiLimits;
use crate::api::responses_tool_repair::ToolCallCache;
use crate::api::responses_tool_repair::value::Value;

pub mod cold;
pub mod decode;
pub mod error;
pub mod merge;
pub mod query;
pub mod replay;

mod tests;

// a LiteLLM_SpendLogs row as B726 reads it; a JSON column holds either the decoded object or its stored text
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct SessionLog {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub proxy_server_request: Option<Json>,
    #[serde(default)]
    pub metadata: Option<Json>,
    #[serde(default)]
    pub response: Option<Json>,
    // request_id, endTime and the other columns only reach the debug record
    #[serde(flatten)]
    pub columns: IndexMap<String, Json>,
}

// B726 runs the parameterized statement against LiteLLM_SpendLogs and returns B858's rows
pub trait SpendLogQuery {
    fn session_logs(
        &self,
        sql: &'static str,
        request_id: &str,
    ) -> impl Future<Output = Result<Vec<SessionLog>, TableError>> + Send;
}

// B1729's ColdStorageHandler lives in betterlite-server, so the route caller hands its reader in
pub trait ColdStorageReader {
    type Error: Error + Send + Sync + 'static;

    fn proxy_server_request(
        &self,
        object_key: &str,
    ) -> impl Future<Output = Result<Option<IndexMap<String, Json>>, Self::Error>> + Send;
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ChatSession {
    pub messages: Vec<Value>,
    pub litellm_session_id: Option<String>,
}

// the caller has already authorized the response id; this service only reads what the id names
pub struct SessionHistory<'deps, Query, Cold, Sleep> {
    // None when the proxy runs without a database client
    pub repository: Option<&'deps Query>,
    // None unless litellm_settings.cold_storage_custom_logger names a logger
    pub cold_storage: Option<&'deps Cold>,
    pub sleeper: &'deps Sleep,
    pub lookup: SessionLookupSettings,
    pub disable_spend_logs: bool,
    pub limits: ApiLimits,
    pub input_cache: &'deps DualLocalCache,
    pub tool_calls: &'deps (dyn ToolCallCache + Sync),
}
