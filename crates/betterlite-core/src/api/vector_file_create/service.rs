use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_context::resource_config_selection::vector_stores::VectorStoreFilesConfigTag;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::api::vector_file_contracts::records::FileParams;
use indexmap::IndexMap;

use crate::api::vector_file_contracts::FileParamsKind;
use crate::api::vector_file_http_create_list::config::FilesParams;
use crate::api::vector_file_http_create_list::error::FileTransportError;
use crate::api::vector_store_http_write::service::ClientKind;

// the kwargs key each operation's async facade sets, and its sync form's caller may set
pub(crate) const ACREATE_FLAG: &str = "acreate";
pub(crate) const ALIST_FLAG: &str = "alist";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Create,
    List,
}

impl Operation {
    // the word LiteLLM's unsupported-provider ValueError uses
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::List => "list",
        }
    }

    pub(crate) const fn flag(self) -> &'static str {
        match self {
            Self::Create => ACREATE_FLAG,
            Self::List => ALIST_FLAG,
        }
    }

    pub(crate) const fn kind(self) -> FileParamsKind {
        match self {
            Self::Create => FileParamsKind::Create,
            Self::List => FileParamsKind::List,
        }
    }
}

// One attachment call as B2480's HTTP handler takes it: the filtered fields, the operation's own overlay and the logging state it reports through
pub struct FileCall<'call> {
    pub operation: Operation,
    pub mode: ClientKind,
    pub provider: ProviderId,
    pub config: VectorStoreFilesConfigTag,
    pub vector_store_id: &'call str,
    pub params: &'call FilesParams,
    // B1071's filter output, with create's file_id argument put back
    pub fields: &'call FileParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    // create forwards only its body overlay, list only its query overlay
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub extra_query: Option<&'call IndexMap<String, Json>>,
    pub timeout: Duration,
    pub logging: &'call Mutex<LoggingState>,
}

// The provider's HTTP attachment service; a local failure and a provider answer both come back as FileTransportError
pub trait FileService: Send + Sync {
    fn file(
        &self,
        call: &FileCall<'_>,
    ) -> impl Future<Output = Result<Json, FileTransportError>> + Send;
}

// LiteLLM's `kwargs.pop("acreate"/"alist", False) is True`: only a boolean true selects the async handler
pub(crate) fn mode(operation: Operation, kwargs: &IndexMap<String, Json>) -> ClientKind {
    if matches!(kwargs.get(operation.flag()), Some(Json::Bool(true))) {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    }
}
