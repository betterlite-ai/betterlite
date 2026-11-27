use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_file_contracts::records::VectorStoreFileUpdateRequest;
use indexmap::IndexMap;

use crate::api::vector_file_http_write::config::WriteParams;
use crate::api::vector_file_http_write::error::FileWriteError;
use crate::api::vector_store_http_write::service::{ClientKind, Operation};

// the kwargs key each file write's async façade sets, and its sync form's caller may set
pub(crate) const UPDATE_FLAG: &str = "aupdate";
pub(crate) const DELETE_FLAG: &str = "adelete";

// One file write as the selected provider's HTTP service takes it: B2402 in production, a recorder in tests
pub struct WriteCall<'call> {
    pub operation: Operation,
    pub mode: ClientKind,
    pub provider: ProviderId,
    pub config: VectorStoreFilesConfigTag,
    pub vector_store_id: &'call str,
    pub file_id: &'call str,
    pub params: &'call WriteParams,
    // update's attributes after the explicit argument is restored, empty for a delete
    pub update_request: &'call VectorStoreFileUpdateRequest,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub timeout: Duration,
    pub logging: &'call Mutex<LoggingState>,
}

// The provider's HTTP write service; a local failure and a provider answer both come back as FileWriteError
pub trait WriteService: Send + Sync {
    fn write(
        &self,
        call: &WriteCall<'_>,
    ) -> impl Future<Output = Result<Json, FileWriteError>> + Send;
}

// LiteLLM's `kwargs.pop(flag, False) is True`: only a boolean true selects the async handler, and the pop takes the flag whatever its value
pub(crate) fn take_mode(flag: &str, kwargs: &mut IndexMap<String, Json>) -> ClientKind {
    if matches!(kwargs.shift_remove(flag), Some(Json::Bool(true))) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}

pub(crate) const fn flag(operation: Operation) -> &'static str {
    match operation {
        Operation::Update => UPDATE_FLAG,
        Operation::Delete => DELETE_FLAG,
    }
}
