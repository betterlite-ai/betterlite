use std::collections::BTreeMap;
use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::api::vector_file_contracts::records::VectorStoreFileUpdateRequest;
use http::{HeaderMap as HttpHeaders, Method};
use indexmap::IndexMap;
use url::Url;

use crate::api::vector_store_http_write::error::status_facts;
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, Operation, SuppliedClient,
};

use self::facts::Sent;
use self::request::{request_parts, select_client, validated_headers, wire_headers};
use super::config::{FileWriteConfig, WriteParams};
use super::error::FileWriteError;

mod facts;
mod proxied;
mod request;

// httpx's request as the handler builds it: a body for an update, query params for a delete, and the timeout it was handed
pub struct WriteRequest {
    pub method: Method,
    pub url: Url,
    pub query: Vec<(String, String)>,
    pub headers: HttpHeaders,
    pub body: RequestBody,
    pub timeout: Option<Duration>,
}

// The selected httpx client class: the sync and async forms differ in the timeout they carry and the one they catch
pub trait WriteTransport: Send + Sync {
    fn default_timeout(&self) -> Duration;

    fn send(
        &self,
        request: WriteRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub struct WriteDeps<'deps, C: FileWriteConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct WriteTarget<'call> {
    pub vector_store_id: &'call str,
    pub file_id: &'call str,
    pub params: &'call WriteParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

pub struct UpdateCall<'call> {
    pub target: WriteTarget<'call>,
    pub update_request: &'call VectorStoreFileUpdateRequest,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub struct DeleteCall<'call> {
    pub target: WriteTarget<'call>,
}

pub enum WriteCall<'call> {
    Update(UpdateCall<'call>),
    Delete(DeleteCall<'call>),
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

pub struct PreparedWrite<'deps, C: FileWriteConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    request: WriteRequest,
    operation: Operation,
    mode: ClientKind,
}

// The four handlers' shared front half: client selection, environment validation, the collection base, the transform, the pre-call and the wire form
pub fn prepare<'borrow, 'deps, C, S>(
    deps: &'borrow WriteDeps<'deps, C, S>,
    mode: ClientKind,
    call: &WriteCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedWrite<'borrow, C, S>, FileWriteError>
where
    'deps: 'borrow,
    C: FileWriteConfig,
    S: ClientSource,
{
    let (target, operation) = match call {
        WriteCall::Update(update) => (&update.target, Operation::Update),
        WriteCall::Delete(delete) => (&delete.target, Operation::Delete),
    };
    let client = select_client(deps, mode, target.params.ssl_verify)?;
    let headers = validated_headers(deps.config, target);
    let api_base = deps
        .config
        .complete_url(target.params, target.vector_store_id)?;
    let parts = request_parts(deps.config, call, &api_base, &headers, log)?;
    Ok(PreparedWrite {
        config: deps.config,
        client,
        request: WriteRequest {
            method: http_method(operation),
            url: parts.url,
            query: parts.query,
            headers: wire_headers(&headers)?,
            body: parts.body,
            timeout: target.timeout,
        },
        operation,
        mode,
    })
}

const fn http_method(operation: Operation) -> Method {
    match operation {
        Operation::Update => Method::POST,
        Operation::Delete => Method::DELETE,
    }
}

// One send, no retry: a transport failure becomes a provider error, a status outside 200..300 becomes one too, and only then does the response transform run
pub async fn send<C, S>(prepared: PreparedWrite<'_, C, S>) -> Result<Json, FileWriteError>
where
    C: FileWriteConfig,
    S: ClientSource<Client: WriteTransport>,
{
    let PreparedWrite {
        config,
        client,
        request,
        operation,
        mode,
    } = prepared;
    let given = request.timeout;
    let timeout = given.unwrap_or_else(|| client.get().default_timeout());
    let started = Instant::now();
    let reply = client.get().send(request).await;
    let sent = Sent {
        mode,
        operation,
        given,
        timeout,
        elapsed: started.elapsed(),
    };
    let answer = reply.map_err(|error| {
        FileWriteError::Provider(facts::provider_error(
            config,
            &facts::transport_facts(&error, &sent),
        ))
    })?;
    answer_outcome(config, operation, &answer)
}

// The response transform runs after the send catch: raise_for_status lets only a 2xx through, and every other status is _handle_error's
fn answer_outcome<C>(
    config: &C,
    operation: Operation,
    answer: &UpstreamResponse,
) -> Result<Json, FileWriteError>
where
    C: FileWriteConfig,
{
    if (200..300).contains(&answer.status) {
        let response = facts::http_response(answer);
        return match operation {
            Operation::Update => config
                .transform_update_response(&response)
                .map_err(FileWriteError::Provider),
            Operation::Delete => config
                .transform_delete_response(&response)
                .map_err(FileWriteError::Provider),
        };
    }
    Err(FileWriteError::Provider(facts::provider_error(
        config,
        &status_facts(answer),
    )))
}
