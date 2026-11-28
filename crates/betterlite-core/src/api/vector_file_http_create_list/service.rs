use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::FileParams;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use http::{HeaderMap as HttpHeaders, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use self::facts::{Attempt, send_facts};
use self::request::{call_timeout, endpoint, validated_headers, wire_headers, wire_request};
use super::config::{FilesConfig, FilesParams};
use super::error::FileTransportError;
use super::payload;
use crate::api::vector_store_http_write::error::{handle_error, status_facts};
use crate::api::vector_store_http_write::service::{ClientKind, ClientSource, SuppliedClient};

mod facts;
mod proxied;
mod request;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Create,
    List,
}

impl Operation {
    const fn http_method(self) -> Method {
        match self {
            Self::Create => Method::POST,
            Self::List => Method::GET,
        }
    }
}

pub struct FileRequest {
    pub method: Method,
    pub url: Url,
    pub query: Vec<(String, String)>,
    pub headers: HttpHeaders,
    pub body: RequestBody,
    pub timeout: Option<Duration>,
}

// The selected httpx client class; only a create send carries a timeout, so only it can time out as litellm.Timeout
pub trait FileTransport: Send + Sync {
    fn send(
        &self,
        request: FileRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub struct FileDeps<'deps, C: FilesConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct FileTarget<'call> {
    pub vector_store_id: &'call str,
    pub params: &'call FilesParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

pub struct CreateCall<'call> {
    pub target: FileTarget<'call>,
    pub create_request: &'call FileParams,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub struct ListCall<'call> {
    pub target: FileTarget<'call>,
    pub query_params: &'call FileParams,
    pub extra_query: Option<&'call IndexMap<String, Json>>,
}

pub enum FileCall<'call> {
    Create(CreateCall<'call>),
    List(ListCall<'call>),
}

enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

impl<Client> Selected<'_, Client> {
    const fn get(&self) -> &Client {
        match self {
            Self::Supplied(client) => client,
            Self::Created(client) => client,
        }
    }
}

impl<C, S> FileDeps<'_, C, S>
where
    C: FilesConfig,
    S: ClientSource,
{
    fn select(
        &self,
        mode: ClientKind,
        ssl_verify: Option<bool>,
    ) -> Result<Selected<'_, S::Client>, FileTransportError> {
        match &self.supplied {
            Some(supplied) if supplied.kind == mode => Ok(Selected::Supplied(supplied.transport)),
            Some(_) | None => Ok(Selected::Created(self.source.client(ssl_verify)?)),
        }
    }
}

pub struct PreparedFile<'deps, C: FilesConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    request: FileRequest,
    operation: Operation,
    mode: ClientKind,
}

// The four handlers' shared front half: client selection, environment validation, the provider URL, the transformed fields, the pre-call and the wire request
pub fn prepare<'borrow, 'deps, C, S>(
    deps: &'borrow FileDeps<'deps, C, S>,
    mode: ClientKind,
    call: &FileCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedFile<'borrow, C, S>, FileTransportError>
where
    'deps: 'borrow,
    C: FilesConfig,
    S: ClientSource,
{
    let (target, operation) = match call {
        FileCall::Create(create) => (&create.target, Operation::Create),
        FileCall::List(list) => (&list.target, Operation::List),
    };
    let client = deps.select(mode, target.params.ssl_verify)?;
    let headers = validated_headers(deps.config, target);
    let (api_base, url) = endpoint(deps.config, target)?;
    let fields = payload::fields(call)?;
    log_pre_call(&api_base, &headers, &fields, log)?;
    let timeout = call_timeout(operation, target.timeout);
    Ok(PreparedFile {
        config: deps.config,
        client,
        request: wire_request(operation, url, &fields, wire_headers(&headers)?, timeout)?,
        operation,
        mode,
    })
}

// LiteLLM logs input="", api_key="" and the transformed fields as complete_input_dict, before the send
fn log_pre_call(
    api_base: &str,
    headers: &OpenAiHeaders,
    fields: &IndexMap<String, Json>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), FileTransportError> {
    let logged = headers
        .iter()
        .map(|(name, value)| (name.clone(), Json::Text(value.expose_secret().to_owned())))
        .collect();
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: Some(RawBody::Object(fields.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env)
        .map(drop)
        .map_err(FileTransportError::Logging)
}

// One send, no retry: a failed send and a create status of 400 or more become provider errors; a list status never does
pub async fn send<C, S>(prepared: PreparedFile<'_, C, S>) -> Result<Json, FileTransportError>
where
    C: FilesConfig,
    S: ClientSource<Client: FileTransport>,
{
    let PreparedFile {
        config,
        client,
        request,
        operation,
        mode,
    } = prepared;
    let timeout = request.timeout;
    let started = Instant::now();
    let reply = client.get().send(request).await;
    let attempt = Attempt {
        operation,
        mode,
        timeout,
        elapsed: started.elapsed(),
    };
    let answer = reply.map_err(|error| {
        FileTransportError::Provider(handle_error(&send_facts(&error, &attempt), None))
    })?;
    if operation == Operation::Create && answer.status >= 400 {
        return Err(FileTransportError::Provider(handle_error(
            &status_facts(&answer),
            None,
        )));
    }
    config
        .transform_response(&answer)
        .map_err(FileTransportError::Provider)
}
