use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Mutex;
use std::time::Duration;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::identity::provider_id::ProviderId;
use indexmap::IndexMap;

use crate::api::vector_store_http_create_read::config::CreateReadConfig;
use crate::api::vector_store_http_create_read::error::{CapabilityError, CreateReadError};
use crate::api::vector_store_http_write::config::{WriteConfig, WriteParams};
use crate::api::vector_store_http_write::service::{ClientKind, SuppliedClient};
use crate::api::vector_store_search::provider::ResolvedProvider;

// the kwargs key each operation's async façade sets, and its sync form's caller may set
pub(crate) const RETRIEVE_FLAG: &str = "aretrieve";
pub(crate) const LIST_FLAG: &str = "alist";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Retrieve,
    List,
}

impl Operation {
    // the word LiteLLM's unsupported-provider ValueError uses
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Retrieve => "retrieve",
            Self::List => "list",
        }
    }

    // the kwargs flag whose boolean true selects the async handler
    pub(crate) const fn flag(self) -> &'static str {
        match self {
            Self::Retrieve => RETRIEVE_FLAG,
            Self::List => LIST_FLAG,
        }
    }
}

// B2485 takes a sized config, so the box B010's selection built is lent back as one
pub struct ReadConfig<'config>(pub Box<dyn CreateReadConfig + 'config>);

impl CreateReadConfig for ReadConfig<'_> {
    fn validate_create(&self) -> Result<(), CapabilityError> {
        self.0.validate_create()
    }

    fn provider(&self) -> &dyn WriteConfig {
        self.0.provider()
    }
}

// B010's config source, handed the provider and api_type B174 resolved
pub trait VectorStoreReadConfigs: Send + Sync {
    fn read_config<'source>(
        &'source self,
        resolved: &ResolvedProvider,
    ) -> Option<ReadConfig<'source>>;
}

// the four list arguments as B2485's list query takes them, each left out when it is None
#[derive(Clone, Copy, Default)]
pub struct ListQuery<'call> {
    pub after: Option<&'call str>,
    pub before: Option<&'call str>,
    pub limit: Option<i64>,
    pub order: Option<&'call str>,
}

// one read as the provider's HTTP service takes it: the resolved provider and its configuration, the mode, the arguments, the caller's timeout and the supplied client
pub struct ReadCall<'call, Client> {
    pub operation: Operation,
    pub mode: ClientKind,
    pub provider: ProviderId,
    pub config: ReadConfig<'call>,
    pub vector_store_id: Option<&'call str>,
    pub query: ListQuery<'call>,
    pub params: &'call WriteParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
    pub timeout: Duration,
    pub client: Option<&'call SuppliedClient<'call, Client>>,
    pub logging: &'call Mutex<LoggingState>,
}

// The provider's HTTP read: B2485 in production, a recorder in tests
pub trait ReadProvider: Send + Sync {
    type Client: Send + Sync;

    fn read(
        &self,
        call: &ReadCall<'_, Self::Client>,
    ) -> impl Future<Output = Result<Json, CreateReadError>> + Send;
}

// LiteLLM's `kwargs.pop("aretrieve", False) is True`: only a boolean true selects the async handler
pub(crate) fn mode(operation: Operation, kwargs: &IndexMap<String, Json>) -> ClientKind {
    if matches!(kwargs.get(operation.flag()), Some(Json::Bool(true))) {
        return ClientKind::Asynchronous;
    }
    ClientKind::Sync
}
