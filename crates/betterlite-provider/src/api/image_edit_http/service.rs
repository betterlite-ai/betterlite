use std::future::Future;
use std::time::{Duration, Instant};

use betterlite_core::api::vector_store_http_write::error::{ClientError, handle_error};
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::RequestBody;
use betterlite_http::proxy::transport_error::TransportError;
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::call_helpers::headers::{
    AdditionalHeaders, ProviderHeaderProjection, set_provider_response_headers,
};
use betterlite_openai::api::image_contracts::records::{
    ImageFile, ImageFiles, ImageHeaders, ImageParams, ImageResponse,
};
use http::HeaderMap;
use url::Url;
use uuid::Uuid;

use super::error::{ImageEditHttpError, RequestError};
use super::facts::{self, Failure, SendError, Sent};
use super::request::{self, Fields, Logged};
use crate::api::image_contracts::edit::{
    BaseImageEditConfig, ImageEditEnvironment, ImageEditReply, ImageEditRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

// One image edit call's inputs: the fields the transforms read, the caller's headers and the request's timeout
pub struct ImageEditCall<'call> {
    pub model: &'call str,
    pub prompt: Option<&'call str>,
    pub image: Option<&'call ImageFile>,
    pub optional_params: &'call ImageParams,
    pub litellm_params: &'call ImageParams,
    pub extra_headers: Option<&'call ImageHeaders>,
    pub timeout: Duration,
}

pub struct EditRequest {
    pub url: Url,
    pub headers: HeaderMap,
    pub body: RequestBody,
    pub timeout: Duration,
}

// The selected httpx client class: the sync and async forms differ in the timeout they carry and the one they catch
pub trait EditTransport: Send + Sync {
    fn send(
        &self,
        request: EditRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send;
}

pub trait ClientSource: Send + Sync {
    type Client: EditTransport;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Self::Client, ClientError>;
}

pub struct ImageEditDeps<'deps, C: BaseImageEditConfig, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

pub struct ImageEditOutcome {
    pub response: ImageResponse,
    pub provider_headers: ProviderHeaderProjection,
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

// The handler's shared body: the request built through the config, then one send
pub(super) async fn run<C, S>(
    mode: ClientKind,
    deps: &ImageEditDeps<'_, C, S>,
    call: &ImageEditCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
where
    C: BaseImageEditConfig,
    S: ClientSource,
{
    let (client, request) = prepare(mode, deps, call, log).await?;
    send(deps.config, client.get(), request, mode, call.model).await
}

// Client selection, the environment headers and their explicit override, the resolved URL before the transform, the finalizer over its exact output, the pre-call, then the wire request
async fn prepare<'borrow, 'deps, C, S>(
    mode: ClientKind,
    deps: &'borrow ImageEditDeps<'deps, C, S>,
    call: &ImageEditCall<'_>,
    log: &mut LogContext<'_, '_>,
) -> Result<(Selected<'borrow, S::Client>, EditRequest), ImageEditHttpError<C::Error>>
where
    'deps: 'borrow,
    C: BaseImageEditConfig,
    S: ClientSource,
{
    let fields = request::fields(call.litellm_params)?;
    let caller = request::caller_headers(call.optional_params)?;
    let client = select(deps, mode, fields.ssl_verify)?;
    let headers = environment_headers(deps.config, call, &fields, &caller);
    let resolved = complete_url(deps.config, call, &fields)?;
    let (data, files) = transform(deps.config, mode, call, &headers).await?;
    let finalized = deps
        .config
        .finalize_image_edit_request_data(data, &resolved);
    let logged = Logged {
        prompt: call.prompt,
        api_base: &resolved,
        headers: &headers,
        data: &finalized,
        files: &files,
    };
    request::log_pre_call(logged, mode, log)?;
    let request = EditRequest {
        url: request::endpoint(&resolved)?,
        headers: request::wire_headers(&headers)?,
        body: request_body(deps.config, &finalized, &files)?,
        timeout: call.timeout,
    };
    Ok((client, request))
}

// A supplied client serves only the mode it declares; the pool answers every other call
fn select<'borrow, 'deps, C, S>(
    deps: &'borrow ImageEditDeps<'deps, C, S>,
    mode: ClientKind,
    ssl_verify: Option<bool>,
) -> Result<Selected<'borrow, S::Client>, ClientError>
where
    'deps: 'borrow,
    C: BaseImageEditConfig,
    S: ClientSource,
{
    match deps.supplied.as_ref().filter(|client| client.kind == mode) {
        Some(client) => Ok(Selected::Supplied(client.transport)),
        None => Ok(Selected::Created(deps.source.client(ssl_verify)?)),
    }
}

// The environment hook completes the caller's own headers, and the explicit map overwrites what it returns
fn environment_headers<C: BaseImageEditConfig>(
    config: &C,
    call: &ImageEditCall<'_>,
    fields: &Fields,
    caller: &ImageHeaders,
) -> ImageHeaders {
    let validated = config.validate_environment(ImageEditEnvironment {
        headers: caller,
        model: call.model,
        api_key: fields.api_key.as_ref(),
        litellm_params: Some(call.litellm_params),
        api_base: fields.api_base.as_deref(),
    });
    request::merge(validated, call.extra_headers)
}

fn complete_url<C: BaseImageEditConfig>(
    config: &C,
    call: &ImageEditCall<'_>,
    fields: &Fields,
) -> Result<String, ImageEditHttpError<C::Error>> {
    config
        .get_complete_url(call.model, fields.api_base.as_deref(), call.litellm_params)
        .map_err(|source| ImageEditHttpError::Config { source })
}

// The mode picks the transform: the sync hook or the awaited async one over the same request
async fn transform<C: BaseImageEditConfig>(
    config: &C,
    mode: ClientKind,
    call: &ImageEditCall<'_>,
    headers: &ImageHeaders,
) -> Result<(ImageParams, ImageFiles), ImageEditHttpError<C::Error>> {
    let edit = ImageEditRequest {
        model: call.model,
        prompt: call.prompt,
        image: call.image,
        optional_params: call.optional_params.clone(),
        litellm_params: call.litellm_params,
        headers: headers.clone(),
    };
    let transformed = match mode {
        ClientKind::Sync => config.transform_image_edit_request(edit),
        ClientKind::Asynchronous => config.async_transform_image_edit_request(edit).await,
    };
    transformed.map_err(|source| ImageEditHttpError::Config { source })
}

// The body the config asks for: the finalized data as form fields beside the transform's files, or the finalized data as JSON
fn request_body<C: BaseImageEditConfig>(
    config: &C,
    data: &ImageParams,
    files: &ImageFiles,
) -> Result<RequestBody, ImageEditHttpError<C::Error>> {
    if !config.use_multipart_form_data() {
        let encoded = serde_json::to_vec(data).map_err(RequestError::Encode)?;
        return Ok(RequestBody::Json(encoded));
    }
    Ok(RequestBody::Multipart {
        boundary: Uuid::new_v4().to_string(),
        parts: request::multipart_parts(data, files)?,
    })
}

// One send, no retry: a transport failure becomes a provider error, and a status of 400 or more becomes one too
async fn send<C, Client>(
    config: &C,
    client: &Client,
    request: EditRequest,
    mode: ClientKind,
    model: &str,
) -> Result<ImageEditOutcome, ImageEditHttpError<C::Error>>
where
    C: BaseImageEditConfig,
    Client: EditTransport,
{
    let timeout = request.timeout;
    let started = Instant::now();
    let reply = client.send(request).await;
    let sent = Sent {
        mode,
        timeout,
        elapsed: started.elapsed(),
    };
    let answer = match reply {
        Ok(answer) => answer,
        Err(error) => {
            let failure = match &error {
                TransportError::ConnectTimeout(_) | TransportError::Deadline(_) => {
                    facts::send_failure(SendError::TimedOut, sent)
                }
                other => facts::send_failure(SendError::Failed(other), sent),
            };
            return Err(provider(config, &failure));
        }
    };
    if answer.status >= 400 {
        return Err(provider(config, &facts::reply_failure(&answer)));
    }
    let raw = facts::image_reply(&answer);
    let response = config
        .transform_image_edit_response(ImageEditReply {
            model,
            raw_response: &raw,
        })
        .map_err(|source| ImageEditHttpError::Config { source })?;
    let provider_headers = set_provider_response_headers(
        &AdditionalHeaders::default(),
        &facts::provider_headers(&answer.headers),
    );
    Ok(ImageEditOutcome {
        response,
        provider_headers,
    })
}

// The config's hook decides the class; the shared converter decides the status, the message and whether the status was synthesized
fn provider<C: BaseImageEditConfig>(config: &C, failure: &Failure) -> ImageEditHttpError<C::Error> {
    let base = handle_error(&failure.facts, None);
    let mut error = config.get_error_class(&base.message, base.status_code, &failure.headers);
    if base.status_code_is_synthesized {
        error.status_code_is_synthesized = true;
    }
    ImageEditHttpError::Provider(Box::new(error))
}
