pub mod error;
pub mod handler;
mod logging;
mod mock;
mod params;
mod prepare;
mod tests;

use std::collections::BTreeMap;
use std::sync::Mutex;

use betterlite_logging::logging_state::LoggingState;
use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::api::container_contracts::records::{
    ContainerListResponse, ContainerObject,
};
use betterlite_openai::containers::container_file_http::request::ParamValue;
use betterlite_openai::openai_base_config::Object;
use betterlite_redact::secret::Redactor;
use indexmap::IndexMap;
use secrecy::SecretString;

use self::error::ContainerOpFailure;
use self::handler::ContainerCollectionHandler;
use self::prepare::create::{PreparedCreate, finish_create, prepare_create};
use self::prepare::list::{PreparedList, finish_list, prepare_list};
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;

// LiteLLM declares both providers and the timeout argument with these defaults
pub const DEFAULT_PROVIDER: &str = "openai";
pub const DEFAULT_TIMEOUT_SECONDS: f64 = 600.0;
pub(crate) const CREATE_CALL_TYPE: &str = "create_container";
pub(crate) const LIST_CALL_TYPE: &str = "list_containers";

// every keyword create_container takes, typed the way the caller passes it
#[derive(Clone, Debug)]
pub struct ContainerCreateCall {
    pub name: String,
    // the JSON map of an expires_after object
    pub expires_after: Option<Object>,
    pub file_ids: Option<Vec<String>>,
    // an explicit zero is falsy in LiteLLM's `timeout or DEFAULT_REQUEST_TIMEOUT`, so it falls back too
    pub timeout: Option<f64>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub custom_llm_provider: String,
    pub extra_headers: Option<BTreeMap<String, String>>,
    // accepted and never sent on create
    pub extra_query: Option<IndexMap<String, ParamValue>>,
    pub extra_body: Option<BTreeMap<String, String>>,
    pub mock_response: Option<Json>,
    pub litellm_metadata: Option<IndexMap<String, JsonValue>>,
    pub litellm_call_id: Option<String>,
    // every other keyword, which reads only as litellm_params extras and logging metadata
    pub extra: IndexMap<String, Json>,
}

// every keyword list_containers takes, typed the way the caller passes it
#[derive(Clone, Debug)]
pub struct ContainerListCall {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub order: Option<String>,
    pub timeout: Option<f64>,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub custom_llm_provider: String,
    pub extra_headers: Option<BTreeMap<String, String>>,
    pub extra_query: Option<IndexMap<String, ParamValue>>,
    // accepted and never sent on list
    pub extra_body: Option<BTreeMap<String, String>>,
    pub mock_response: Option<Json>,
    pub litellm_metadata: Option<IndexMap<String, JsonValue>>,
    pub litellm_call_id: Option<String>,
    pub extra: IndexMap<String, Json>,
}

// both signatures declare the same defaults: the openai provider and 600 seconds
macro_rules! container_call_default {
    ($call:ident { $($field:ident: $value:expr),* $(,)? }) => {
        impl Default for $call {
            fn default() -> Self {
                Self {
                    $($field: $value,)*
                }
            }
        }
    };
}

container_call_default!(ContainerCreateCall {
    name: String::new(),
    expires_after: None,
    file_ids: None,
    timeout: Some(DEFAULT_TIMEOUT_SECONDS),
    api_key: None,
    api_base: None,
    api_version: None,
    custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
    extra_headers: None,
    extra_query: None,
    extra_body: None,
    mock_response: None,
    litellm_metadata: None,
    litellm_call_id: None,
    extra: IndexMap::new(),
});

container_call_default!(ContainerListCall {
    after: None,
    limit: None,
    order: None,
    timeout: Some(DEFAULT_TIMEOUT_SECONDS),
    api_key: None,
    api_base: None,
    api_version: None,
    custom_llm_provider: DEFAULT_PROVIDER.to_owned(),
    extra_headers: None,
    extra_query: None,
    extra_body: None,
    mock_response: None,
    litellm_metadata: None,
    litellm_call_id: None,
    extra: IndexMap::new(),
});

// the collaborators the operations need: B247's logging object, B185's redactor and REQUEST_TIMEOUT's seconds
#[derive(Clone, Copy)]
pub struct ContainerCallDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    // LiteLLM's request_timeout constant that REQUEST_TIMEOUT overrides, used only where LiteLLM reaches it
    pub request_timeout_seconds: f64,
}

// the four entry points differ only in the call, the reply, the handler method and whether the handler answer is awaited
macro_rules! container_operations {
    (
        sync $name:ident,
        $call:ident,
        $reply:ty,
        $prepared:ident,
        $prepare:path,
        $method:ident,
        $finish:path
    ) => {
        pub fn $name<Handler, Configs>(
            handler: &Handler,
            configs: &Configs,
            deps: ContainerCallDeps<'_>,
            call: &$call,
        ) -> Result<$reply, ContainerOpFailure>
        where
            Handler: ContainerCollectionHandler,
            Configs: ContainerConfigLookup,
        {
            let ready = match $prepare(configs, &deps, call)? {
                $prepared::Mock(answer) => return Ok(answer),
                $prepared::Call(ready) => ready,
            };
            $finish(
                handler.$method(&ready.handler_call(call, deps.logging)),
                call,
                deps.redactor,
            )
        }
    };
    (
        asynchronous $name:ident,
        $call:ident,
        $reply:ty,
        $prepared:ident,
        $prepare:path,
        $method:ident,
        $finish:path
    ) => {
        pub async fn $name<Handler, Configs>(
            handler: &Handler,
            configs: &Configs,
            deps: ContainerCallDeps<'_>,
            call: &$call,
        ) -> Result<$reply, ContainerOpFailure>
        where
            Handler: ContainerCollectionHandler,
            Configs: ContainerConfigLookup,
        {
            let ready = match $prepare(configs, &deps, call)? {
                $prepared::Mock(answer) => return Ok(answer),
                $prepared::Call(ready) => ready,
            };
            $finish(
                handler
                    .$method(&ready.handler_call(call, deps.logging))
                    .await,
                call,
                deps.redactor,
            )
        }
    };
}

// LiteLLM's create_container: mock first, then the blocking handler runs the call it prepared
container_operations!(
    sync create_container,
    ContainerCreateCall,
    ContainerObject,
    PreparedCreate,
    prepare_create,
    create,
    finish_create
);

// LiteLLM's acreate_container: the same steps through the async handler
container_operations!(
    asynchronous acreate_container,
    ContainerCreateCall,
    ContainerObject,
    PreparedCreate,
    prepare_create,
    acreate,
    finish_create
);

// LiteLLM's list_containers: mock first, then the blocking handler answers as is
container_operations!(
    sync list_containers,
    ContainerListCall,
    ContainerListResponse,
    PreparedList,
    prepare_list,
    list,
    finish_list
);

// LiteLLM's alist_containers: the same steps through the async handler
container_operations!(
    asynchronous alist_containers,
    ContainerListCall,
    ContainerListResponse,
    PreparedList,
    prepare_list,
    alist,
    finish_list
);
