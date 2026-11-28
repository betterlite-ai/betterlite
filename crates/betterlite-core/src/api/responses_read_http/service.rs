use std::collections::BTreeMap;
use std::future::Future;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::response_events::RawResponse;
use betterlite_openai::response_resources::request::ListInputItemsQuery;
use betterlite_openai::response_resources::response::TypedReply;
use http::HeaderMap as HttpHeaders;
use http::Method;
use indexmap::IndexMap;
use url::Url;

use self::facts::{body_text, status_facts, transport_facts};
use self::request::{query_pairs, request_endpoint, select_client, wire_headers};
use super::config::{ReadConfig, ReadParams, ReadResource};
use super::error::ReadError;
use crate::api::vector_store_http_write::error::{FailureFacts, handle_error, header_text};
use crate::api::vector_store_http_write::service::ClientSource;

mod facts;
mod proxied;
mod request;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Retrieve,
    InputItems,
}

impl Operation {
    const fn client_label(self) -> &'static str {
        match self {
            Self::Retrieve => "get_responses",
            Self::InputItems => "list_input_items",
        }
    }
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

// A read carries no body and no per-call timeout, whatever the handlers were offered
pub struct ReadRequest {
    pub method: Method,
    pub url: Url,
    pub query: Vec<(String, String)>,
    pub headers: HttpHeaders,
    pub body: RequestBody,
}

// The selected httpx client class: the async form logs a client creation line the sync form does not
pub trait ReadTransport: Send + Sync {
    fn send(
        &self,
        request: ReadRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub struct ReadDeps<'deps, C: ReadConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct ReadTarget<'call> {
    pub response_id: &'call str,
    pub params: &'call ReadParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

pub struct RetrieveCall<'call> {
    pub target: ReadTarget<'call>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub struct InputItemsCall<'call> {
    pub target: ReadTarget<'call>,
    pub query: &'call ListInputItemsQuery,
}

pub enum ReadCall<'call> {
    Retrieve(RetrieveCall<'call>),
    InputItems(InputItemsCall<'call>),
}

impl ReadCall<'_> {
    const fn target(&self) -> &ReadTarget<'_> {
        match self {
            Self::Retrieve(retrieve) => &retrieve.target,
            Self::InputItems(items) => &items.target,
        }
    }

    const fn operation(&self) -> Operation {
        match self {
            Self::Retrieve(_) => Operation::Retrieve,
            Self::InputItems(_) => Operation::InputItems,
        }
    }
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

#[derive(Debug)]
pub enum ReadOutcome {
    Response(Box<TypedReply>),
    InputItems(Json),
}

pub struct PreparedRead<'deps, C: ReadConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    request: ReadRequest,
    operation: Operation,
    mode: ClientKind,
}

// The four handlers' shared front half: client selection, environment validation, the base plus encoded id, the query, the pre-call and the wire request
pub fn prepare<'borrow, 'deps, C, S>(
    deps: &'borrow ReadDeps<'deps, C, S>,
    mode: ClientKind,
    call: &ReadCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedRead<'borrow, C, S>, ReadError>
where
    'deps: 'borrow,
    C: ReadConfig,
    S: ClientSource,
{
    let target = call.target();
    let operation = call.operation();
    let client = select_client(deps, mode, target.params.ssl_verify, operation)?;
    // LiteLLM validates the caller's map in place, and its later update re-applies that same map
    let caller = target.extra_headers.cloned().unwrap_or_default();
    let headers = deps.config.validate_environment(caller, target.params);
    let (api_base, resource) = request_endpoint(deps.config, call)?;
    let ReadResource { url, query } = resource;
    log_pre_call(&api_base, &headers, &query, log)?;
    let pairs = query_pairs(&query)?;
    let request_url = Url::parse(&url).map_err(|source| ReadError::RequestUrl { url, source })?;
    Ok(PreparedRead {
        config: deps.config,
        client,
        request: ReadRequest {
            method: Method::GET,
            url: request_url,
            query: pairs,
            headers: wire_headers(&headers)?,
            body: RequestBody::Empty,
        },
        operation,
        mode,
    })
}

fn log_pre_call(
    api_base: &str,
    headers: &BTreeMap<String, String>,
    query: &IndexMap<String, Json>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), ReadError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.clone()));
    }
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: Some(RawBody::Object(query.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}

// One GET, no retry: a status outside 2xx raises the response itself, and the body is converted only after that catch
pub async fn send<C, S>(prepared: PreparedRead<'_, C, S>) -> Result<ReadOutcome, ReadError>
where
    C: ReadConfig,
    S: ClientSource<Client: ReadTransport>,
{
    let PreparedRead {
        config,
        client,
        request,
        operation,
        mode,
    } = prepared;
    let answer = client
        .get()
        .send(request)
        .await
        .map_err(|error| provider_failure(&transport_facts(&error), mode, operation))?;
    if !(200_u16..300).contains(&answer.status) {
        return Err(provider_failure(&status_facts(&answer), mode, operation));
    }
    let headers: IndexMap<String, String> = answer
        .headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
        .collect();
    let text = body_text(&answer.body);
    let reply = RawResponse {
        status: answer.status,
        headers: &headers,
        text: &text,
    };
    match operation {
        Operation::Retrieve => config
            .retrieve_reply(&reply)
            .map(|typed| ReadOutcome::Response(Box::new(typed)))
            .map_err(ReadError::Response),
        Operation::InputItems => config
            .input_items_reply(&reply)
            .map(ReadOutcome::InputItems)
            .map_err(ReadError::Response),
    }
}

fn provider_failure(facts: &FailureFacts, mode: ClientKind, operation: Operation) -> ReadError {
    let error = handle_error(facts, None);
    // LiteLLM logs this failure only in the async retrieve handler
    if mode == ClientKind::Asynchronous && operation == Operation::Retrieve {
        tracing::debug!("Error retrieving response: {error}");
    }
    ReadError::Provider(error)
}
