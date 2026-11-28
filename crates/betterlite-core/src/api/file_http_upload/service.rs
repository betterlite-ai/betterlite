use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_openai::files::records::FileCreate;
use futures_util::Stream;
use http::{HeaderMap as HttpHeaders, Method};
use tokio_util::task::TaskTracker;
use url::Url;

use crate::api::api_boundaries::ApiLimits;
use crate::api::vector_store_http_write::error::header_text;
use crate::api::vector_store_http_write::service::ClientSource;

use super::config::{FileParams, FileResponse, FileUploadConfig, Headers, UploadParams};
use super::error::UploadError;
use super::plan::UploadPlan;

mod exchange;
mod pre_call;
mod proxied;
mod wire;

use exchange::{Exchange, Reply};
use pre_call::{caller_headers, log_pre_call};
use wire::traditional_body;

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

pub struct UploadDeps<'deps, C: FileUploadConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
    pub tasks: &'deps TaskTracker,
    pub boundaries: &'deps (dyn Fn() -> String + Send + Sync),
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
    boundaries: &'deps (dyn Fn() -> String + Send + Sync),
    timeout: Option<Duration>,
    limits: ApiLimits,
}

// create_file's front half: credentials, the complete URL, the transformed plan, the client, the async pre_call
pub fn prepare<'deps, 'env, C, S>(
    deps: &'deps UploadDeps<'deps, C, S>,
    mode: Mode,
    call: &UploadCall<'_>,
    log: &'deps mut LogContext<'_, 'env>,
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
    S: ClientSource<Client: UploadTransport>,
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
    let reply = run_plan(&mut exchange, plan).await?;
    let response = upload_response(reply);
    let transformed_params = result_params(mode, &params, &api_base);
    config
        .transform_create_file_response(&response, &transformed_params)
        .map_err(UploadError::Provider)
}

// The plan's shape picks the branch; only a sync byte body is decoded before it is sent
async fn run_plan<C, Client>(
    exchange: &mut Exchange<'_, '_, C, Client>,
    plan: UploadPlan,
) -> Result<Reply, UploadError>
where
    C: FileUploadConfig,
    Client: UploadTransport,
{
    match plan {
        UploadPlan::TwoStep(two_step) => exchange.two_step(&two_step).await,
        UploadPlan::Presigned(presigned) => exchange.presigned(&presigned).await,
        UploadPlan::Media(media) => exchange.media(*media).await,
        UploadPlan::Text(text) => {
            exchange
                .base_body(UploadBody::Bytes(text.into_bytes()))
                .await
        }
        UploadPlan::Bytes(bytes) => {
            let body = traditional_body(exchange.mode, &bytes)?;
            exchange.base_body(UploadBody::Bytes(body)).await
        }
        UploadPlan::Multipart(multipart) => exchange.multipart(multipart.parts).await,
        UploadPlan::Unsupported(_) => Err(UploadError::UnsupportedPlan),
    }
}

fn upload_response(reply: Reply) -> FileResponse {
    FileResponse {
        status_code: reply.response.status,
        headers: reply
            .response
            .headers
            .iter()
            .map(|(name, value)| (name.as_str().to_owned(), header_text(value)))
            .collect(),
        body: reply.response.body,
        url: Some(reply.url),
    }
}

// the sync handler hands the transform a copy with api_base as upload_url when it set none
fn result_params(mode: Mode, params: &FileParams, api_base: &str) -> FileParams {
    let mut transformed = params.clone();
    if mode == Mode::Sync && !transformed.contains_key("upload_url") {
        transformed.insert("upload_url".to_owned(), Json::Text(api_base.to_owned()));
    }
    transformed
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
