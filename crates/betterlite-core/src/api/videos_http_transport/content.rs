use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_state::LoggingState;
use betterlite_openai::error::ProviderError;
use secrecy::SecretString;

use super::client::{
    ClientKind, ClientRequest, ClientSource, HttpVerb, Selected, Sent, ServeBody, VideoDeps, build,
    exchange, select_client,
};
use super::error::VideoTransportError;
use super::params;
use super::video_config::{
    ErrorFacts, Headers, Params, VideoEnvironment, VideoHttpConfig, VideoLogging,
    VideoRequestContext,
};

// The caller's side of LiteLLM's video_content, whose timeout argument no send of it ever uses
pub struct VideoContentCall<'call> {
    pub video_id: &'call str,
    pub custom_llm_provider: &'call str,
    pub litellm_params: &'call Params,
    pub extra_headers: Option<&'call Headers>,
    pub api_key: Option<&'call SecretString>,
    pub variant: Option<&'call str>,
    pub timeout: Duration,
}

struct Content<'deps, C, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    headers: Headers,
    url: String,
    data: Params,
}

// The content download: a POST when the transform returned params, else a GET, with no per-send timeout either way
pub async fn video_content_handler<C, S>(
    deps: &VideoDeps<'_, C, S>,
    call: &VideoContentCall<'_>,
    log: &mut LoggingState,
    is_async: bool,
) -> Result<Vec<u8>, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
    S: ClientSource,
{
    let kind = if is_async {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    let content = prepare_content(deps, call, kind)?;
    let response = send_content(&content, kind).await?;
    transform_content(content.config, log, kind, &response).await
}

fn prepare_content<'deps, C, S>(
    deps: &'deps VideoDeps<'deps, C, S>,
    call: &VideoContentCall<'_>,
    kind: ClientKind,
) -> Result<Content<'deps, C, S>, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
    S: ClientSource,
{
    let client = select_client(
        deps,
        ClientRequest {
            kind,
            ssl_verify: params::ssl_verify(call.litellm_params)?,
            custom_llm_provider: call.custom_llm_provider,
        },
    )?;
    let headers = content_headers(deps.config, call);
    let base = content_base(deps.config, call)?;
    let (url, data) = content_transform(deps.config, call, &base, &headers)?;
    Ok(Content {
        config: deps.config,
        client,
        headers,
        url,
        data,
    })
}

// validate_environment is handed the call's extra headers and then has them laid over its answer
fn content_headers<C>(config: &C, call: &VideoContentCall<'_>) -> Headers
where
    C: VideoHttpConfig,
{
    let caller = call.extra_headers.cloned().unwrap_or_default();
    let mut headers = config.validate_environment(VideoEnvironment {
        headers: &caller,
        model: "",
        api_key: call.api_key,
        litellm_params: Some(call.litellm_params),
    });
    if let Some(extra) = call.extra_headers {
        headers.extend(extra.clone());
    }
    headers
}

fn content_base<C>(
    config: &C,
    call: &VideoContentCall<'_>,
) -> Result<String, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
{
    config
        .get_complete_url(
            "",
            params::api_base(call.litellm_params)?,
            call.litellm_params,
        )
        .map_err(VideoTransportError::Config)
}

fn content_transform<C>(
    config: &C,
    call: &VideoContentCall<'_>,
    api_base: &str,
    headers: &Headers,
) -> Result<(String, Params), VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
{
    config
        .transform_video_content_request(
            call.video_id,
            call.variant,
            VideoRequestContext {
                api_base,
                litellm_params: call.litellm_params,
                headers,
            },
        )
        .map_err(VideoTransportError::Config)
}

// httpx's get sends the params of an empty body as query parameters and raises on no status; a send timeout is the client's
async fn send_content<C, S>(
    content: &Content<'_, C, S>,
    kind: ClientKind,
) -> Result<UpstreamResponse, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
    S: ClientSource,
{
    let (verb, body) = if content.data.is_empty() {
        (HttpVerb::Get, ServeBody::Empty)
    } else {
        (HttpVerb::Post, ServeBody::Json(&content.data))
    };
    let request = build(verb.method(), &content.url, &content.headers, body, None);
    let sent = Sent {
        mode: kind,
        verb,
        timeout: None,
    };
    exchange(content.config, content.client.get(), request, sent).await
}

// The response transform stays inside the send's try there, so this is where its error and a failing status meet
async fn transform_content<C>(
    config: &C,
    log: &mut LoggingState,
    kind: ClientKind,
    response: &UpstreamResponse,
) -> Result<Vec<u8>, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
{
    let mut logging = VideoLogging {
        model_call_details: log.model_call_details.clone(),
    };
    let outcome = match kind {
        ClientKind::Sync => config.transform_video_content_response(response, &mut logging),
        ClientKind::Asynchronous => {
            config
                .async_transform_video_content_response(response, &mut logging)
                .await
        }
    };
    // the logging object is live in LiteLLM, so what a failing transform wrote stays for the error path
    log.model_call_details = logging.model_call_details;
    outcome.map_err(|error| VideoTransportError::Provider(Box::new(wrapped(config, &error))))
}

// _handle_error over a transform's own exception: its status if it has one, a synthesized 500 when it does not
fn wrapped<C>(config: &C, error: &C::Error) -> ProviderError
where
    C: VideoHttpConfig,
{
    let ErrorFacts {
        message,
        status_code,
        headers,
    } = config.error_facts(error);
    let status = status_code.unwrap_or(500);
    let mut provider = config.get_error_class(&message, status, &headers);
    provider.status_code_is_synthesized = status_code.is_none();
    provider
}
