use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::FileCreate;
use futures_util::Stream;
use http::header::{HeaderName, HeaderValue};
use http::{HeaderMap as HttpHeaders, Method};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use tokio_util::task::TaskTracker;
use url::Url;

use crate::api::api_boundaries::ApiLimits;
use crate::api::vector_store_http_write::error::{ClientError, header_text};

use super::config::{FileParams, FileResponse, FileUploadConfig, Headers, UploadParams};
use super::error::UploadError;
use super::plan::UploadPlan;

mod exchange;
mod proxied;

use exchange::{Exchange, Reply, traditional_body};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Sync,
    Asynchronous,
}

pub struct SuppliedClient<'client, Client> {
    pub mode: Mode,
    pub transport: &'client Client,
}

// The body as the selected client sends it; a text or bytes body carries no default content type
pub enum UploadBody {
    Bytes(Vec<u8>),
    Multipart {
        boundary: String,
        parts: Vec<MultipartPart>,
    },
    Blocks(BlockStream),
}

pub type BlockStream = Pin<Box<dyn Stream<Item = Vec<u8>> + Send>>;

pub struct UploadRequest {
    pub method: Method,
    pub url: Url,
    pub headers: HttpHeaders,
    pub body: UploadBody,
    pub timeout: Option<Duration>,
}

// The selected httpx client class; both forms read the whole answer before anyone looks at its status
pub trait UploadTransport: Send + Sync {
    fn send(
        &self,
        request: UploadRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub trait ClientSource: Send + Sync {
    type Client: UploadTransport;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Self::Client, ClientError>;
}

pub struct UploadDeps<'deps, C: FileUploadConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
    pub tasks: &'deps TaskTracker,
    pub boundaries: &'deps dyn Fn() -> String,
    pub limits: ApiLimits,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct UploadCall<'call> {
    pub data: &'call FileCreate,
    pub params: &'call UploadParams<'call>,
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

pub struct PreparedUpload<'deps, 'env, C: FileUploadConfig, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    plan: UploadPlan,
    api_base: String,
    headers: Headers,
    params: FileParams,
    mode: Mode,
    env: &'deps PrePostEnv<'env>,
    tasks: &'deps TaskTracker,
    boundaries: &'deps dyn Fn() -> String,
    timeout: Option<Duration>,
    limits: ApiLimits,
}

// create_file's front half: credentials, the complete URL, the transformed plan, the client, and the
// pre_call only the async handler records
pub fn prepare<'deps, 'state, 'env, C, S>(
    deps: &'deps UploadDeps<'deps, C, S>,
    mode: Mode,
    call: &UploadCall<'_>,
    log: &'deps mut LogContext<'state, 'env>,
) -> Result<PreparedUpload<'deps, 'env, C, S>, UploadError>
where
    C: FileUploadConfig,
    S: ClientSource,
{
    let params = call.params;
    let headers = deps
        .config
        .validate_environment(caller_headers(call.data), params);
    let api_base = deps
        .config
        .get_complete_file_url(params, call.data)?
        .ok_or(UploadError::ApiBaseRequired)?;
    let plan = deps
        .config
        .transform_create_file_request(call.data, params)?;
    let client = select_client(deps, mode, params.ssl_verify)?;
    if mode == Mode::Asynchronous {
        log_pre_call(&api_base, &headers, &plan, log)?;
    }
    if let UploadPlan::Unsupported(_) = plan {
        return Err(UploadError::UnsupportedPlan);
    }
    Ok(PreparedUpload {
        config: deps.config,
        client,
        plan,
        api_base,
        headers,
        params: params.litellm_params.clone(),
        mode,
        env: log.env,
        tasks: deps.tasks,
        boundaries: deps.boundaries,
        timeout: call.data.timeout,
        limits: deps.limits,
    })
}

// One exchange, no retry: the plan picks the branch, and only the branches LiteLLM wraps convert a failure
pub async fn send<C, S>(prepared: PreparedUpload<'_, '_, C, S>) -> Result<Json, UploadError>
where
    C: FileUploadConfig,
    S: ClientSource,
{
    let PreparedUpload {
        config,
        client,
        plan,
        api_base,
        headers,
        mut params,
        mode,
        env,
        tasks,
        boundaries,
        timeout,
        limits,
    } = prepared;
    let mut exchange = Exchange {
        config,
        client: client.get(),
        api_base: &api_base,
        headers: &headers,
        params: &mut params,
        env,
        mode,
        tasks,
        boundaries,
        timeout,
        limits,
    };
    let reply = match plan {
        UploadPlan::TwoStep(plan) => exchange.two_step(&plan).await?,
        UploadPlan::Presigned(plan) => exchange.presigned(&plan).await?,
        UploadPlan::Media(plan) => exchange.media(*plan).await?,
        UploadPlan::Text(text) => exchange.traditional(text.into_bytes()).await?,
        UploadPlan::Bytes(bytes) => {
            exchange
                .traditional(traditional_body(mode, &bytes)?)
                .await?
        }
        UploadPlan::Multipart(plan) => exchange.multipart(plan.parts).await?,
        UploadPlan::Unsupported(_) => return Err(UploadError::UnsupportedPlan),
    };
    transform_reply(&exchange, reply)
}

// The sync handler hands transform_create_file_response a copy with api_base as upload_url when the
// transform set none; the async handler hands its own litellm_params
fn transform_reply<C, S>(
    exchange: &Exchange<'_, '_, C, S>,
    reply: Reply,
) -> Result<Json, UploadError>
where
    C: FileUploadConfig,
    S: ClientSource,
{
    let response = FileResponse {
        status_code: reply.response.status,
        headers: reply
            .response
            .headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
            .collect(),
        body: reply.response.body,
        url: Some(reply.url),
    };
    let mut params = exchange.params.clone();
    if exchange.mode == Mode::Sync && !params.contains_key("upload_url") {
        params.insert(
            "upload_url".to_owned(),
            Json::Text(exchange.api_base.to_owned()),
        );
    }
    exchange
        .config
        .transform_create_file_response(&response, &params)
        .map_err(UploadError::Provider)
}

fn select_client<'borrow, 'deps, C, S>(
    deps: &'borrow UploadDeps<'deps, C, S>,
    mode: Mode,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, UploadError>
where
    'deps: 'borrow,
    C: FileUploadConfig,
    S: ClientSource,
{
    if let Some(supplied) = &deps.supplied
        && supplied.mode == mode
    {
        return Ok(Selected::Supplied(supplied.transport));
    }
    Ok(Selected::Created(deps.source.client(ssl_verify)?))
}

fn caller_headers(data: &FileCreate) -> Headers {
    data.extra_headers
        .iter()
        .map(|(name, value)| (name.clone(), SecretString::from(value.as_str())))
        .collect()
}

fn log_pre_call(
    api_base: &str,
    headers: &Headers,
    plan: &UploadPlan,
    log: &mut LogContext<'_, '_>,
) -> Result<(), UploadError> {
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
            complete_input_dict: plan.logged_input().map(raw_body),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env)?;
    // LiteLLM passes api_key="" beside input, and B249's pre_call record has no member to carry it
    log.state
        .model_call_details
        .insert("api_key".to_owned(), Json::Text(String::new()));
    Ok(())
}

// a plan logs an object or a placeholder text; any other JSON keeps the shape it was built with
fn raw_body(value: Json) -> RawBody {
    match value {
        Json::Object(fields) => RawBody::Object(fields),
        Json::Text(text) => RawBody::Text(text),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::List(_) => {
            RawBody::Text(serde_json::to_string(&value).unwrap_or_default())
        }
    }
}

pub(super) fn headers_from<'pair, H>(pairs: H) -> Result<HttpHeaders, UploadError>
where
    H: Iterator<Item = (&'pair str, &'pair str)>,
{
    let mut map = HttpHeaders::new();
    for (name, value) in pairs {
        let header_name =
            HeaderName::from_bytes(name.as_bytes()).map_err(|source| UploadError::HeaderName {
                name: name.to_owned(),
                source,
            })?;
        let header_value =
            HeaderValue::from_str(value).map_err(|source| UploadError::HeaderValue {
                name: name.to_owned(),
                source,
            })?;
        map.insert(header_name, header_value);
    }
    Ok(map)
}

pub(super) fn request_url(text: &str) -> Result<Url, UploadError> {
    Url::parse(text).map_err(|source| UploadError::RequestUrl {
        url: text.to_owned(),
        source,
    })
}
