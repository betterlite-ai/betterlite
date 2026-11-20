use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::VectorStoresConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;

use crate::api::vector_store_http_write::config::WriteParams;
use crate::api::vector_store_http_write::error::WriteError;
use crate::api::vector_store_http_write::params::UpdateParams;
use crate::api::vector_store_http_write::service::ClientKind;

// the kwargs key each operation's async façade sets, and its sync form's caller may set
pub(crate) const UPDATE_FLAG: &str = "aupdate";
pub(crate) const DELETE_FLAG: &str = "adelete";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Update,
    Delete,
}

impl Operation {
    // the word LiteLLM's unsupported-provider ValueError uses
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Update => "update",
            Self::Delete => "delete",
        }
    }
}

// One write as the selected provider's HTTP service takes it: B2402 in production, a recorder in tests
pub struct WriteCall<'call> {
    pub operation: Operation,
    pub mode: ClientKind,
    pub provider: ProviderId,
    pub config: VectorStoresConfigTag,
    pub vector_store_id: &'call str,
    pub params: &'call WriteParams,
    // LiteLLM's create-parameter filter output, empty for a delete
    pub optional_params: &'call UpdateParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub timeout: Duration,
    pub logging: &'call Mutex<LoggingState>,
}

// The provider's HTTP write service; a local failure and a provider answer both come back as WriteError
pub trait WriteService: Send + Sync {
    fn write(&self, call: &WriteCall<'_>) -> impl Future<Output = Result<Json, WriteError>> + Send;
}

// LiteLLM's `kwargs.pop("aupdate", False) is True`: only a boolean true selects the async handler
pub(crate) fn mode(operation: Operation, kwargs: &IndexMap<String, Json>) -> ClientKind {
    let flag = match operation {
        Operation::Update => UPDATE_FLAG,
        Operation::Delete => DELETE_FLAG,
    };
    if matches!(kwargs.get(flag), Some(Json::Bool(true))) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}
