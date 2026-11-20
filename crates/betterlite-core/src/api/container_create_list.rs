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

use self::error::{ContainerOpError, ContainerOpFailure, failure};
use self::handler::ContainerCollectionHandler;
use self::params::extra_body_json;
use self::prepare::create::{PreparedCreate, prepare_create};
use self::prepare::list::{PreparedList, prepare_list};
use crate::api::container_collection_http::ContainerCollectionError;
use crate::api::container_file_endpoints::handler::ContainerConfigLookup;
// ast-grep-ignore: no-long-names
use crate::containers::container_request_parameters::encode_container_id_in_response;

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

impl Default for ContainerCreateCall {
    fn default() -> Self {
        Self {
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
        }
    }
}

impl Default for ContainerListCall {
    fn default() -> Self {
        Self {
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
        }
    }
}

// the collaborators the operations need: B247's logging object, B185's redactor and REQUEST_TIMEOUT's seconds
#[derive(Clone, Copy)]
pub struct ContainerCallDeps<'deps> {
    pub logging: &'deps Mutex<LoggingState>,
    pub redactor: &'deps Redactor,
    // LiteLLM's request_timeout constant that REQUEST_TIMEOUT overrides, used only where LiteLLM reaches it
    pub request_timeout_seconds: f64,
}

// LiteLLM's create_container: mock first, then the blocking handler runs the call it prepared
pub fn create_container<Handler, Configs>(
    handler: &Handler,
    configs: &Configs,
    deps: ContainerCallDeps<'_>,
    call: &ContainerCreateCall,
) -> Result<ContainerObject, ContainerOpFailure>
where
    Handler: ContainerCollectionHandler,
    Configs: ContainerConfigLookup,
{
    let ready = match prepare_create(configs, &deps, call)? {
        PreparedCreate::Mock(container) => return Ok(container),
        PreparedCreate::Call(ready) => ready,
    };
    let created = handler
        .create(&ready.handler_call(call, deps.logging))
        .map_err(|error| handler_failure(error, &call.custom_llm_provider, deps.redactor))?;
    encode_created(created, call, deps.redactor)
}

// LiteLLM's acreate_container: the same steps through the async handler, encoded after it finishes
pub async fn acreate_container<Handler, Configs>(
    handler: &Handler,
    configs: &Configs,
    deps: ContainerCallDeps<'_>,
    call: &ContainerCreateCall,
) -> Result<ContainerObject, ContainerOpFailure>
where
    Handler: ContainerCollectionHandler,
    Configs: ContainerConfigLookup,
{
    let ready = match prepare_create(configs, &deps, call)? {
        PreparedCreate::Mock(container) => return Ok(container),
        PreparedCreate::Call(ready) => ready,
    };
    let created = handler
        .acreate(&ready.handler_call(call, deps.logging))
        .await
        .map_err(|error| handler_failure(error, &call.custom_llm_provider, deps.redactor))?;
    encode_created(created, call, deps.redactor)
}

// LiteLLM's list_containers: mock first, then the blocking handler answers as is
pub fn list_containers<Handler, Configs>(
    handler: &Handler,
    configs: &Configs,
    deps: ContainerCallDeps<'_>,
    call: &ContainerListCall,
) -> Result<ContainerListResponse, ContainerOpFailure>
where
    Handler: ContainerCollectionHandler,
    Configs: ContainerConfigLookup,
{
    let ready = match prepare_list(configs, &deps, call)? {
        PreparedList::Mock(response) => return Ok(response),
        PreparedList::Call(ready) => ready,
    };
    handler
        .list(&ready.handler_call(call, deps.logging))
        .map_err(|error| handler_failure(error, &call.custom_llm_provider, deps.redactor))
}

// LiteLLM's alist_containers: the same steps through the async handler
pub async fn alist_containers<Handler, Configs>(
    handler: &Handler,
    configs: &Configs,
    deps: ContainerCallDeps<'_>,
    call: &ContainerListCall,
) -> Result<ContainerListResponse, ContainerOpFailure>
where
    Handler: ContainerCollectionHandler,
    Configs: ContainerConfigLookup,
{
    let ready = match prepare_list(configs, &deps, call)? {
        PreparedList::Mock(response) => return Ok(response),
        PreparedList::Call(ready) => ready,
    };
    handler
        .alist(&ready.handler_call(call, deps.logging))
        .await
        .map_err(|error| handler_failure(error, &call.custom_llm_provider, deps.redactor))
}

// _encode_created_container_id's encode step: the object as returned, with a routed id
fn encode_created(
    container: ContainerObject,
    call: &ContainerCreateCall,
    redactor: &Redactor,
) -> Result<ContainerObject, ContainerOpFailure> {
    // ast-grep-ignore: no-long-names
    let encoded = encode_container_id_in_response(
        &container.id,
        Some(&call.custom_llm_provider),
        call.litellm_metadata.as_ref(),
        extra_body_json(call.extra_body.as_ref()).as_ref(),
    )
    .map_err(|error| {
        failure(
            ContainerOpError::Encode(error),
            &call.custom_llm_provider,
            redactor,
        )
    })?;
    Ok(ContainerObject {
        id: encoded,
        ..container
    })
}

fn handler_failure(
    error: ContainerCollectionError,
    provider: &str,
    redactor: &Redactor,
) -> ContainerOpFailure {
    failure(ContainerOpError::Handler(error), provider, redactor)
}
