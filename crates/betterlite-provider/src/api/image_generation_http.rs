use std::error::Error;
use std::time::{Duration, Instant};

use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_models::call_helpers::headers::ProviderHeaderProjection;
use betterlite_openai::api::image_contracts::records::{
    ImageHeaders, ImageHttpResponse, ImageParams, ImageResponse, ImageUsage,
};
use betterlite_openai::image_generation::transport::ImageGenerationTransport;

use self::error::{ConfigFailure, ImageGenerationError};
use self::log::{ImageGenerationLogger, PreCallRecord};
use crate::api::image_contracts::generation::{BaseImageGenerationConfig, ImageGenerationReply};

pub mod error;
mod facts;
pub mod log;
mod request;
mod tests;
mod wire;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientKind {
    Sync,
    Asynchronous,
}

// The API key the handler was handed and the one litellm_params carries reach the config separately
pub struct ImageGenerationCall<'call> {
    pub model: &'call str,
    pub prompt: &'call str,
    pub optional_params: &'call ImageParams,
    pub litellm_params: &'call ImageParams,
    pub extra_headers: Option<&'call ImageHeaders>,
    // accepted for the handler's signature and never merged into this request
    pub extra_body: Option<&'call ImageParams>,
    pub fake_stream: bool,
    pub litellm_metadata: Option<&'call ImageParams>,
    pub api_key: Option<&'call str>,
    pub timeout: Duration,
}

pub struct SuppliedClient<'client, Client> {
    pub kind: ClientKind,
    pub transport: &'client Client,
}

pub trait ClientSource: Send + Sync {
    type Client: ImageGenerationTransport;
    type Error: Error + Send + Sync + 'static;

    fn client(&self, ssl_verify: Option<bool>) -> Result<Self::Client, Self::Error>;
}

pub struct ImageGenerationDeps<'deps, C, S: ClientSource> {
    pub config: &'deps C,
    pub source: &'deps S,
    pub supplied: Option<SuppliedClient<'deps, S::Client>>,
}

enum Selected<'client, Client> {
    Supplied(&'client Client),
    Created(Client),
}

pub struct PreparedGeneration<'prepared, 'call, C, S: ClientSource> {
    config: &'prepared C,
    client: Selected<'prepared, S::Client>,
    call: &'call ImageGenerationCall<'call>,
    data: ImageParams,
    request: OutboundRequest,
    mode: ClientKind,
}

#[derive(Debug)]
pub struct ImageGenerationOutcome {
    pub response: ImageResponse,
    pub provider_headers: ProviderHeaderProjection,
}

// The handler's front half, in LiteLLM's order: client, validated headers plus the caller's, the URL, the body, the pre-call
pub fn prepare<'borrow, 'deps, 'call, C, S>(
    deps: &'borrow ImageGenerationDeps<'deps, C, S>,
    mode: ClientKind,
    call: &'call ImageGenerationCall<'call>,
    logger: &mut dyn ImageGenerationLogger,
) -> Result<PreparedGeneration<'borrow, 'call, C, S>, ImageGenerationError>
where
    'deps: 'borrow,
    C: BaseImageGenerationConfig,
    S: ClientSource,
{
    let client = request::select(deps, mode, call)?;
    let headers = request::validated_headers(deps.config, call)?;
    let (api_base, url) = request::endpoint(deps.config, call)?;
    let data = request::transform(deps.config, call, &headers)?;
    logger.pre_call(PreCallRecord {
        input: call.prompt,
        api_key: "",
        api_base: &api_base,
        headers: &headers,
        complete_input_dict: &data,
    });
    let request = wire::outbound(
        &url,
        &headers,
        &data,
        call.timeout,
        deps.config.use_multipart_form_data(),
    )?;
    Ok(PreparedGeneration {
        config: deps.config,
        client,
        call,
        data,
        request,
        mode,
    })
}

// One send, no retry: a transport or status failure becomes a provider error, and the response transform runs after both
pub async fn send<C, S>(
    prepared: PreparedGeneration<'_, '_, C, S>,
) -> Result<ImageGenerationOutcome, ImageGenerationError>
where
    C: BaseImageGenerationConfig,
    S: ClientSource,
{
    let PreparedGeneration {
        config,
        client,
        call,
        data,
        request,
        mode,
    } = prepared;
    let timeout = request.deadline;
    let started = Instant::now();
    let transport: &S::Client = match &client {
        Selected::Supplied(supplied) => *supplied,
        Selected::Created(supplied) => supplied,
    };
    let reply = transport.generate(request).await;
    let sent = facts::Sent {
        mode,
        timeout,
        elapsed: started.elapsed(),
    };
    let answer = reply.map_err(|error| facts::transport_error(&error, &sent, config))?;
    if answer.status >= 400 {
        return Err(facts::status_error(&answer, config));
    }
    let response = facts::http_response(&answer)?;
    let image = transform_response(config, call, &data, &response)?;
    Ok(ImageGenerationOutcome {
        response: image,
        provider_headers: facts::provider_headers(&response.headers),
    })
}

fn transform_response<C>(
    config: &C,
    call: &ImageGenerationCall<'_>,
    data: &ImageParams,
    response: &ImageHttpResponse,
) -> Result<ImageResponse, ImageGenerationError>
where
    C: BaseImageGenerationConfig,
{
    let mut target = ImageResponse {
        created: 0,
        data: Some(Vec::new()),
        usage: Some(ImageUsage::default()),
    };
    config
        .transform_image_generation_response(ImageGenerationReply {
            model: call.model,
            raw_response: response,
            model_response: &mut target,
            request_data: data,
            optional_params: call.optional_params,
            litellm_params: call.litellm_params,
            api_key: None,
            json_mode: None,
        })
        .map_err(|source| {
            ImageGenerationError::Config(ConfigFailure {
                source: Box::new(source),
            })
        })
}
