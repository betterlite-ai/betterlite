use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_models::decisions::Json;
use betterlite_openai::vector_stores::Headers as OpenAiHeaders;
use betterlite_openai::vector_stores::store::CreateParams;
use http::Method;
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use url::Url;

use self::facts::Sent;
use self::request::{
    call_timeout, create_endpoint, list_endpoint, list_params, retrieve_endpoint, select_client,
    validated_headers,
};
use self::wire::wire_headers;
use super::config::CreateReadConfig;
use super::error::CreateReadError;
use crate::api::vector_store_http_write::config::{WriteConfig, WriteParams};
use crate::api::vector_store_http_write::error::{WriteError, handle_error};
use crate::api::vector_store_http_write::service::{
    ClientKind, ClientSource, LogContext, SuppliedClient, WriteRequest, WriteTransport,
};

mod facts;
mod request;
mod wire;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operation {
    Create,
    Retrieve,
    List,
}

impl Operation {
    const fn http_method(self) -> Method {
        match self {
            Self::Create => Method::POST,
            Self::Retrieve | Self::List => Method::GET,
        }
    }
}

// The litellm_params, caller headers and timeout the three handlers share
pub struct CreateReadTarget<'call> {
    pub params: &'call WriteParams,
    pub extra_headers: Option<&'call BTreeMap<String, String>>,
    pub timeout: Option<Duration>,
}

pub struct CreateCall<'call> {
    pub target: CreateReadTarget<'call>,
    pub optional_params: &'call CreateParams,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub struct RetrieveCall<'call> {
    pub target: CreateReadTarget<'call>,
    pub vector_store_id: &'call str,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

// The four list fields, each left out of both the logged map and the query when it is None
#[derive(Default)]
pub struct ListQuery<'call> {
    pub after: Option<&'call str>,
    pub before: Option<&'call str>,
    pub limit: Option<i64>,
    pub order: Option<&'call str>,
}

pub struct ListCall<'call> {
    pub target: CreateReadTarget<'call>,
    pub query: ListQuery<'call>,
    pub extra_body: Option<&'call IndexMap<String, Json>>,
}

pub enum CreateReadCall<'call> {
    Create(CreateCall<'call>),
    Retrieve(RetrieveCall<'call>),
    List(ListCall<'call>),
}

pub struct CreateReadDeps<'deps, C: CreateReadConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
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

pub struct PreparedCall<'deps, C: CreateReadConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    request: WriteRequest,
    operation: Operation,
    mode: ClientKind,
}

// The six handlers' shared front half: client selection, the create-only capability check, headers, the URL and body, the pre-call and the wire request
pub fn prepare<'borrow, 'deps, C, S>(
    deps: &'borrow CreateReadDeps<'deps, C, S>,
    mode: ClientKind,
    call: &CreateReadCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<PreparedCall<'borrow, C, S>, CreateReadError>
where
    'deps: 'borrow,
    C: CreateReadConfig,
    S: ClientSource,
{
    let (target, operation) = match call {
        CreateReadCall::Create(create) => (&create.target, Operation::Create),
        CreateReadCall::Retrieve(retrieve) => (&retrieve.target, Operation::Retrieve),
        CreateReadCall::List(list) => (&list.target, Operation::List),
    };
    let client = select_client(deps, mode, target.params.ssl_verify)?;
    // only the create handlers ask whether the provider can create, and they ask before any header is built
    if operation == Operation::Create {
        deps.config.validate_create()?;
    }
    let provider = deps.config.provider();
    let headers = validated_headers(provider, target);
    let (api_base, url, payload) = endpoint(provider, call)?;
    log_pre_call(&api_base, &headers, call, payload.as_ref(), log)?;
    Ok(PreparedCall {
        config: deps.config,
        client,
        request: WriteRequest {
            method: operation.http_method(),
            url,
            headers: wire_headers(&headers, target.extra_headers)?,
            body: wire_body(payload.as_ref())?,
            timeout: call_timeout(mode, operation, target.timeout),
        },
        operation,
        mode,
    })
}

// The complete base URL, the URL to call and, for a create, the body it sends
type Endpoint = (String, Url, Option<IndexMap<String, Json>>);

fn endpoint(
    provider: &dyn WriteConfig,
    call: &CreateReadCall<'_>,
) -> Result<Endpoint, CreateReadError> {
    match call {
        CreateReadCall::Create(create) => {
            let (api_base, url, body) = create_endpoint(provider, create)?;
            Ok((api_base, url, Some(body)))
        }
        CreateReadCall::Retrieve(retrieve) => {
            let (api_base, url) = retrieve_endpoint(provider, retrieve)?;
            Ok((api_base, url, None))
        }
        CreateReadCall::List(list) => {
            let (api_base, url) = list_endpoint(provider, list)?;
            Ok((api_base, url, None))
        }
    }
}

fn log_pre_call(
    api_base: &str,
    headers: &OpenAiHeaders,
    call: &CreateReadCall<'_>,
    body: Option<&IndexMap<String, Json>>,
    log: &mut LogContext<'_, '_>,
) -> Result<(), CreateReadError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.expose_secret().to_owned()));
    }
    let mut additional = AdditionalArgs {
        api_base: Some(api_base.to_owned()),
        headers: Some(logged),
        complete_input_dict: body.cloned().map(RawBody::Object),
        request_str: None,
        openai_sdk: None,
        passthrough: IndexMap::new(),
    };
    // the four handlers log their own argument map, and the list one is the handler whose map is the query
    if let CreateReadCall::List(list) = call {
        let params = list_params(&list.query);
        additional
            .passthrough
            .insert("params".to_owned(), Json::Object(params));
    }
    let record = PreCall {
        input: Json::Text(String::new()),
        model: None,
        additional_args: additional,
    };
    pre_call(log.state, &record, log.env)
        .map(drop)
        .map_err(WriteError::Logging)?;
    Ok(())
}

fn wire_body(body: Option<&IndexMap<String, Json>>) -> Result<RequestBody, WriteError> {
    let Some(payload) = body else {
        return Ok(RequestBody::Empty);
    };
    serde_json::to_vec(payload)
        .map(RequestBody::Json)
        .map_err(WriteError::Encode)
}

// One send, no retry: a transport failure becomes a provider error, and only a create raises for a status
pub async fn send<C, S>(prepared: PreparedCall<'_, C, S>) -> Result<Json, CreateReadError>
where
    C: CreateReadConfig,
    S: ClientSource<Client: WriteTransport>,
{
    let PreparedCall {
        config,
        client,
        request,
        operation,
        mode,
    } = prepared;
    let provider = config.provider();
    let timeout = request
        .timeout
        .unwrap_or_else(|| client.get().default_timeout());
    let started = Instant::now();
    let reply = client.get().send(request).await;
    let sent = Sent {
        mode,
        operation,
        timeout,
        elapsed: started.elapsed(),
    };
    let answer = reply.map_err(|error| {
        CreateReadError::Write(WriteError::Provider(handle_error(
            &facts::transport_facts(&error, &sent),
            Some(provider),
        )))
    })?;
    decode(provider, operation, &answer)
}

// httpx raises for every status outside 2xx, redirects included, so only a create reads a 3xx as an answer
fn decode(
    provider: &dyn WriteConfig,
    operation: Operation,
    answer: &UpstreamResponse,
) -> Result<Json, CreateReadError> {
    if operation == Operation::Create && !(200..300).contains(&answer.status) {
        return Err(CreateReadError::Write(WriteError::Provider(handle_error(
            &facts::status_facts(answer),
            Some(provider),
        ))));
    }
    match operation {
        Operation::Create | Operation::Retrieve => provider
            .transform_create_response(&facts::http_response(answer))
            .map_err(|error| CreateReadError::Write(WriteError::Provider(error))),
        Operation::List => serde_json::from_slice(&answer.body)
            .map_err(|source| CreateReadError::Write(WriteError::Reply(source))),
    }
}
