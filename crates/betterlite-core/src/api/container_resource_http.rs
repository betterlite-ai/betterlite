#[path = "container_resource_http/tests/delete.rs"]
mod delete;
#[path = "container_resource_http/tests/retrieve.rs"]
mod retrieve;
mod tests;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::{
    api::container_contracts::{
        config::{BaseContainerConfig, ContainerRequestContext},
        error::{BaseLlmException, ContainerError},
        records::{ContainerObject, DeleteContainerResult},
    },
    containers::container_file_http::{
        ContainerFileHttpError, PreCallHook,
        request::{CallParams, ParamValue, pre_call_record, read_params, request_headers},
        response::raise_for_error_status,
        transport::{
            AsyncClient, BlockingClient, ClientSource, ContainerHttpReply, ContainerHttpRequest,
            ContainerMethod, Selected, TransferError,
        },
    },
    openai_base_config::Object,
};
use indexmap::IndexMap;
use serde_json::Value;
use thiserror::Error;
use url::Url;

const TIMEOUT_STATUS: u16 = 408;
const FALLBACK_STATUS: u16 = 500;
const TIMEOUT_MESSAGE: &str = "litellm.Timeout: Connection timed out after None seconds.";

#[derive(Debug, Error)]
pub enum ContainerResourceHttpError {
    #[error(transparent)]
    Call(#[from] ContainerFileHttpError),
    #[error("{exception}")]
    Provider {
        #[source]
        pub exception: BaseLlmException,
        pub status_code_is_synthesized: bool,
    },
    #[error("the {name} query value is not a string, a number or a boolean")]
    QueryValue { pub name: String },
}

pub type RetrieveResult = Result<ContainerObject, ContainerResourceHttpError>;
pub type DeleteResult = Result<DeleteContainerResult, ContainerResourceHttpError>;

#[rustfmt::skip]
trait Resource {
    type Reply;
    const METHOD: ContainerMethod;
    const CAUGHT_TIMEOUT_STATUS: Option<u16>;
    fn transform(config: &dyn BaseContainerConfig, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError>;
    fn decode(config: &dyn BaseContainerConfig, body: &[u8]) -> Result<Self::Reply, ContainerError>;
}

struct Retrieve;
#[rustfmt::skip]
impl Resource for Retrieve {
    type Reply = ContainerObject;
    const METHOD: ContainerMethod = ContainerMethod::Get;
    const CAUGHT_TIMEOUT_STATUS: Option<u16> = None;
    fn transform(config: &dyn BaseContainerConfig, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { config.transform_container_retrieve_request(container_id, api_base, context) }
    fn decode(config: &dyn BaseContainerConfig, body: &[u8]) -> Result<ContainerObject, ContainerError> { config.transform_container_retrieve_response(body) }
}
struct Delete;
#[rustfmt::skip]
impl Resource for Delete {
    type Reply = DeleteContainerResult;
    const METHOD: ContainerMethod = ContainerMethod::Delete;
    const CAUGHT_TIMEOUT_STATUS: Option<u16> = Some(TIMEOUT_STATUS);
    fn transform(config: &dyn BaseContainerConfig, container_id: &str, api_base: &str, context: ContainerRequestContext<'_>) -> Result<(String, Object), ContainerError> { config.transform_container_delete_request(container_id, api_base, context) }
    fn decode(config: &dyn BaseContainerConfig, body: &[u8]) -> Result<DeleteContainerResult, ContainerError> { config.transform_container_delete_response(body) }
}

pub enum Handled<Ready, Pending> {
    Done(Ready),
    Pending(Pending),
}
pub struct ContainerResourceCall<'call, Source: ClientSource> {
    pub container_id: &'call str,
    pub config: &'call dyn BaseContainerConfig,
    pub litellm_params: &'call Object,
    pub logging_obj: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_query: Option<&'call IndexMap<String, ParamValue>>,
    // LiteLLM accepts a timeout and never sends it, so the selected client's own deadline stays
    pub timeout: Option<Duration>,
    pub blocking_client: Option<&'call Source::Blocking>,
    pub async_client: Option<&'call Source::Asynchronous>,
}

pub struct ContainerResourceHandler<Source: ClientSource, Hook: PreCallHook> {
    pub clients: Source,
    pub logging: Hook,
}
// LiteLLM spells the blocking and async entry points of each endpoint out twice; one request shape serves both
macro_rules! endpoints {
    ($($sync:ident, $asynchronous:ident, $kind:ident, $result:ident;)+) => {
        $(
            pub fn $sync(
                &self,
                call: &ContainerResourceCall<'_, Source>,
                is_async: bool,
            ) -> Result<
                Handled<<$kind as Resource>::Reply, impl Future<Output = $result>>,
                ContainerResourceHttpError,
            > {
                if is_async {
                    return Ok(Handled::Pending(self.$asynchronous(call)));
                }
                self.sync_call::<$kind>(call).map(Handled::Done)
            }

            pub async fn $asynchronous(
                &self,
                call: &ContainerResourceCall<'_, Source>,
            ) -> $result {
                let (client, request) = self.async_begin::<$kind>(call)?;
                let reply = AsyncClient::send(client.get(), &request)
                    .await
                    .map_err(|error| transport_failure(&error, call.config, None))?;
                decoded::<$kind>(call.config, &reply)
            }
        )+
    };
}
impl<Source: ClientSource, Hook: PreCallHook> ContainerResourceHandler<Source, Hook> {
    endpoints! {
        container_retrieve_handler, acontainer_retrieve_handler, Retrieve, RetrieveResult;
        container_delete_handler, acontainer_delete_handler, Delete, DeleteResult;
    }

    fn sync_call<Kind: Resource>(
        &self,
        call: &ContainerResourceCall<'_, Source>,
    ) -> Result<Kind::Reply, ContainerResourceHttpError> {
        let params = read_params(call.litellm_params)?;
        let client = Selected::choose(call.blocking_client, || {
            self.clients.blocking_client(params.ssl_verify.as_ref())
        })
        .map_err(ContainerFileHttpError::Client)?;
        let request = self.prepare::<Kind>(call, &params)?;
        let reply = BlockingClient::send(client.get(), &request)
            .map_err(|error| transport_failure(&error, call.config, Kind::CAUGHT_TIMEOUT_STATUS))?;
        decoded::<Kind>(call.config, &reply)
    }

    fn async_begin<'inner, Kind: Resource>(
        &self,
        call: &ContainerResourceCall<'inner, Source>,
    ) -> Result<
        (Selected<'inner, Source::Asynchronous>, ContainerHttpRequest),
        ContainerResourceHttpError,
    > {
        let params = read_params(call.litellm_params)?;
        let client = Selected::choose(call.async_client, || {
            self.clients.async_client(params.ssl_verify.as_ref())
        })
        .map_err(ContainerFileHttpError::Client)?;
        let request = self.prepare::<Kind>(call, &params)?;
        Ok((client, request))
    }

    // LiteLLM's order: provider headers, base URL, transform, extra query, log, then the method's request
    fn prepare<Kind: Resource>(
        &self,
        call: &ContainerResourceCall<'_, Source>,
        params: &CallParams<'_>,
    ) -> Result<ContainerHttpRequest, ContainerResourceHttpError> {
        let headers = request_headers(call.config, call.extra_headers, params.api_key.as_ref());
        let api_base = call
            .config
            .get_complete_url(params.api_base, call.litellm_params)
            .map_err(ContainerFileHttpError::Config)?;
        let context = ContainerRequestContext {
            litellm_params: call.litellm_params,
            headers: &headers,
        };
        let (url, transformed) =
            Kind::transform(call.config, call.container_id, &api_base, context)
                .map_err(ContainerFileHttpError::Config)?;
        let query = rendered_query(&transformed, call.extra_query)?;
        let mut record = pre_call_record(&url, &headers, &query);
        record.additional_args.passthrough.insert(
            "container_id".to_owned(),
            Json::Text(call.container_id.to_owned()),
        );
        let mut state = call
            .logging_obj
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        self.logging
            .pre_call(&mut state, &record)
            .map_err(ContainerFileHttpError::Logging)?;
        drop(state);
        resource_request(Kind::METHOD, url, headers, query)
    }
}

// Extra query values replace a transformed one of the same name, and names the transform did not answer are appended
fn rendered_query(
    transformed: &Object,
    extra: Option<&IndexMap<String, ParamValue>>,
) -> Result<IndexMap<String, String>, ContainerResourceHttpError> {
    let mut query = IndexMap::new();
    for (name, value) in transformed {
        let rendered = match value {
            Value::Null => String::new(),
            Value::String(text) => text.clone(),
            Value::Number(_) | Value::Bool(_) => value.to_string(),
            Value::Array(_) | Value::Object(_) => {
                return Err(ContainerResourceHttpError::QueryValue { name: name.clone() });
            }
        };
        query.insert(name.clone(), rendered);
    }
    for (name, value) in extra.into_iter().flatten() {
        query.insert(name.clone(), value.render());
    }
    Ok(query)
}

// httpx replaces the URL's own query with the params of a non-GET request, while GET merges them in the client
fn resource_request(
    method: ContainerMethod,
    url: String,
    headers: BTreeMap<String, String>,
    query: IndexMap<String, String>,
) -> Result<ContainerHttpRequest, ContainerResourceHttpError> {
    let mut request = ContainerHttpRequest {
        method,
        url,
        headers,
        query: Vec::new(),
        file: None,
    };
    if matches!(method, ContainerMethod::Get) || query.is_empty() {
        request.query = query.into_iter().collect();
        return Ok(request);
    }
    let mut parsed =
        Url::parse(&request.url).map_err(|source| ContainerFileHttpError::BaseUrl {
            url: request.url.clone(),
            source,
        })?;
    parsed.query_pairs_mut().clear().extend_pairs(&query);
    request.url = parsed.to_string();
    Ok(request)
}

// _handle_error: the caught exception's own status when it has one, a synthesized 500 otherwise
fn transport_failure(
    error: &TransferError,
    config: &dyn BaseContainerConfig,
    caught_timeout_status: Option<u16>,
) -> ContainerResourceHttpError {
    let timed_out = matches!(
        error,
        TransferError::Transport(
            TransportError::ConnectTimeout(_)
                | TransportError::Deadline(_)
                | TransportError::Rejected(HttpError::Timeout)
        )
    );
    let (status, message) = caught_timeout_status.filter(|_| timed_out).map_or_else(
        || (None, error.to_string()),
        |status| (Some(status), TIMEOUT_MESSAGE.to_owned()),
    );
    ContainerResourceHttpError::Provider {
        exception: config.get_error_class(
            &message,
            status.unwrap_or(FALLBACK_STATUS),
            &BTreeMap::new(),
        ),
        status_code_is_synthesized: status.is_none(),
    }
}

fn decoded<Kind: Resource>(
    config: &dyn BaseContainerConfig,
    reply: &ContainerHttpReply,
) -> Result<Kind::Reply, ContainerResourceHttpError> {
    raise_for_error_status(config, reply).map_err(|exception| {
        ContainerResourceHttpError::Provider {
            exception,
            status_code_is_synthesized: false,
        }
    })?;
    Kind::decode(config, &reply.body).map_err(ContainerFileHttpError::Config)
}
