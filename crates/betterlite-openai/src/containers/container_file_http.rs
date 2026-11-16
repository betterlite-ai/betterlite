use std::collections::BTreeMap;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_http::proxy::pool::PoolError;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::PreCall;
use betterlite_logging::logging_state::LoggingState;
use betterlite_prompts::media_metadata::FileData;
use betterlite_prompts::media_metadata::error::MediaMetadataError;
use indexmap::IndexMap;
use serde_json::{Map, Value};
use thiserror::Error;

use self::request::{
    CallParams, ParamValue, RequestTarget, SslVerify, assemble_request, build_query, build_url,
    pre_call_record, read_params, request_headers,
};
use self::response::transform_reply;
use self::transport::{
    AsyncClient as _, BlockingClient as _, ClientSource, ContainerHttpRequest, Selected,
    SuppliedClient, TransferError,
};
use crate::api::container_contracts::catalog::{
    CatalogError, ContainerEndpoint, container_endpoints,
};
use crate::api::container_contracts::config::BaseContainerConfig;
use crate::api::container_contracts::error::{BaseLlmException, ContainerError};
use crate::api::container_contracts::records::{
    ContainerFileListResponse, ContainerFileObject, DeleteContainerFileResponse,
};

pub mod proxied;
pub mod request;
pub mod response;
pub mod transport;

mod tests;

#[derive(Debug, Error)]
pub enum ContainerFileHttpError {
    #[error("Unknown endpoint: {name}")]
    UnknownEndpoint { name: String },
    #[error("Unsupported HTTP method: {method}")]
    UnsupportedMethod { method: String },
    #[error(transparent)]
    Provider(#[from] BaseLlmException),
    #[error(transparent)]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Config(ContainerError),
    #[error(transparent)]
    PathSegment(#[from] HttpError),
    #[error("the container base URL {url} is not valid")]
    BaseUrl {
        url: String,
        #[source]
        source: url::ParseError,
    },
    #[error("the {field} parameter must be {expected}")]
    ParamType {
        field: &'static str,
        expected: &'static str,
    },
    #[error(transparent)]
    Client(#[from] PoolError),
    #[error(transparent)]
    Logging(#[from] PrePostError),
    #[error(transparent)]
    Upload(#[from] MediaMetadataError),
    #[error(transparent)]
    Transfer(#[from] TransferError),
    #[error("the container response is not valid JSON")]
    Json {
        #[source]
        source: serde_json::Error,
    },
    #[error("the container response does not match {response_type}")]
    Model {
        response_type: String,
        #[source]
        source: serde_json::Error,
    },
}

// The input B235's extract_file_data reads: bytes, a name, a content type and headers
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerFileUpload {
    pub file_data: FileData,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ContainerFileReply {
    List(ContainerFileListResponse),
    File(ContainerFileObject),
    Deleted(DeleteContainerFileResponse),
    Bytes(Vec<u8>),
    Raw(Value),
}

// B249's pre_call, handed the call's own logging state; B3368's route builds the environment it needs
pub trait PreCallHook: Send + Sync {
    fn pre_call(&self, state: &mut LoggingState, call: &PreCall) -> Result<(), PrePostError>;
}

pub struct ContainerFileHttpCall<'call, Source: ClientSource> {
    pub endpoint_name: &'call str,
    pub config: &'call dyn BaseContainerConfig,
    pub litellm_params: &'call Map<String, Value>,
    pub logging: &'call Mutex<LoggingState>,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub extra_query: Option<&'call IndexMap<String, ParamValue>>,
    // LiteLLM accepts a timeout and never sends it, so the selected client's own deadline stays
    pub timeout: Option<Duration>,
    // path and query values by name: container_id, file_id, after, limit, order
    pub values: &'call BTreeMap<String, ParamValue>,
    pub file: Option<&'call ContainerFileUpload>,
    pub client: Option<SuppliedClient<'call, Source>>,
}

pub struct GenericContainerHandler<Source: ClientSource, Hook: PreCallHook> {
    pub clients: Source,
    pub logging: Hook,
}

fn find_endpoint(name: &str) -> Result<ContainerEndpoint, ContainerFileHttpError> {
    container_endpoints()?
        .into_iter()
        .find(|endpoint| endpoint.name == name || endpoint.async_name == name)
        .ok_or_else(|| ContainerFileHttpError::UnknownEndpoint {
            name: name.to_owned(),
        })
}

impl<Source: ClientSource, Hook: PreCallHook> GenericContainerHandler<Source, Hook> {
    pub fn handle(
        &self,
        call: &ContainerFileHttpCall<'_, Source>,
    ) -> Result<ContainerFileReply, ContainerFileHttpError> {
        let endpoint = find_endpoint(call.endpoint_name)?;
        let params = read_params(call.litellm_params)?;
        let client = self.blocking_for(call, params.ssl_verify.as_ref())?;
        let reply = client
            .get()
            .send(&self.prepare(&endpoint, call, &params)?)?;
        transform_reply(call.config, &endpoint, &reply)
    }

    pub async fn ahandle(
        &self,
        call: &ContainerFileHttpCall<'_, Source>,
    ) -> Result<ContainerFileReply, ContainerFileHttpError> {
        let endpoint = find_endpoint(call.endpoint_name)?;
        let params = read_params(call.litellm_params)?;
        let client = self.async_for(call, params.ssl_verify.as_ref())?;
        let request = self.prepare(&endpoint, call, &params)?;
        let answer = client.get().send(&request).await;
        transform_reply(call.config, &endpoint, &answer?)
    }

    // A supplied client is kept only when it is the blocking kind
    fn blocking_for<'client>(
        &self,
        call: &ContainerFileHttpCall<'client, Source>,
        ssl_verify: Option<&SslVerify>,
    ) -> Result<Selected<'client, Source::Blocking>, PoolError> {
        if let Some(SuppliedClient::Blocking(supplied)) = &call.client {
            return Ok(Selected::Supplied(*supplied));
        }
        self.clients
            .blocking_client(ssl_verify)
            .map(Selected::Created)
    }

    // A supplied client is kept only when it is the async kind
    fn async_for<'client>(
        &self,
        call: &ContainerFileHttpCall<'client, Source>,
        ssl_verify: Option<&SslVerify>,
    ) -> Result<Selected<'client, Source::Asynchronous>, PoolError> {
        match &call.client {
            Some(SuppliedClient::Asynchronous(supplied)) => Ok(Selected::Supplied(*supplied)),
            Some(SuppliedClient::Blocking(_)) | None => {
                self.clients.async_client(ssl_verify).map(Selected::Created)
            }
        }
    }

    // The steps both paths share, in LiteLLM's order: headers, base URL, path, query, log, then the method and the file
    fn prepare(
        &self,
        endpoint: &ContainerEndpoint,
        call: &ContainerFileHttpCall<'_, Source>,
        params: &CallParams<'_>,
    ) -> Result<ContainerHttpRequest, ContainerFileHttpError> {
        let provider_headers = request_headers(call.config, call.extra_headers, params.api_key);
        let api_base = call
            .config
            .get_complete_url(params.api_base, call.litellm_params)
            .map_err(ContainerFileHttpError::Config)?;
        let path_values: Vec<(&str, String)> = endpoint
            .path_params
            .iter()
            .map(|name| {
                let value = call.values.get(name).map(ParamValue::render);
                (name.as_str(), value.unwrap_or_default())
            })
            .collect();
        let url = build_url(&api_base, &endpoint.path, &path_values)?;
        let query = build_query(&endpoint.query_params, call.values, call.extra_query);
        let record = pre_call_record(&url, &provider_headers, &query);
        let mut state = call.logging.lock().unwrap_or_else(PoisonError::into_inner);
        let logged = self.logging.pre_call(&mut state, &record);
        drop(state);
        logged?;
        let target = RequestTarget {
            url,
            headers: provider_headers,
            query,
        };
        assemble_request(endpoint, call.file, target)
    }
}
