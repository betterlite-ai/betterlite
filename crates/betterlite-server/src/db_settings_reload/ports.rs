use std::future::Future;
use std::pin::Pin;

use async_trait::async_trait;
use betterlite_config::sections::settings::JsonValue;
use betterlite_config::sections::websearch_request_hooks::Settings as WebsearchParams;
use betterlite_db::database_faults::DbError;
use betterlite_db::database_faults::reconnect::DbReconnect;

use crate::config_override_support::models::OverrideKind;

// The LiteLLM_Config and LiteLLM_ConfigOverrides rows the tick reads; None is no row or a null column.
#[async_trait]
pub trait SettingsRows: Send + Sync {
    async fn litellm_settings(&self) -> Result<Option<JsonValue>, DbError>;

    async fn config_override(&self, kind: OverrideKind) -> Result<Option<JsonValue>, DbError>;
}

// The registered web search callback: B9823 implements it over B9821's slot and its builder.
pub trait WebsearchInterception: Send + Sync {
    fn is_registered(&self) -> bool;

    fn apply(&self, params: Option<&WebsearchParams>);
}

// The registered semantic filter hook: B138 implements it over B138's slot and B8201's hook.
#[async_trait]
pub trait SemanticFilterReload<Router: ?Sized>: Send + Sync {
    fn is_ready(&self) -> bool;

    async fn rebuild(&self, enabled: bool, router: Option<&Router>) -> bool;
}

// A proxy with no database reconnect handle: a transport fault is reported, never retried.
#[derive(Debug, Default)]
pub struct NoReconnect;

impl DbReconnect for NoReconnect {
    fn reconnect_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn reconnect_lock_timeout_seconds(&self) -> Option<f64> {
        None
    }

    fn attempt_db_reconnect<'client>(
        &'client mut self,
        _reason: &'client str,
        _timeout_seconds: f64,
        _lock_timeout_seconds: f64,
    ) -> Pin<Box<dyn Future<Output = Result<bool, DbError>> + Send + 'client>> {
        Box::pin(async { Ok(false) })
    }
}
