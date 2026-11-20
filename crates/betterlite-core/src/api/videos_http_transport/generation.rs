use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_logging::logging_pre_post::args::{AdditionalArgs, RawBody};
use betterlite_logging::logging_pre_post::env::PrePostEnv;
use betterlite_logging::logging_pre_post::error::PrePostError;
use betterlite_logging::logging_pre_post::pre_call::{PreCall, pre_call};
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use http::Method;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::client::{
    ClientKind, ClientRequest, ClientSource, HttpVerb, Selected, Sent, ServeBody, VideoDeps,
    VideoTransport, build, exchange, select_client,
};
use super::error::{BadField, SendFailure, VideoTransportError};
use super::multipart;
use super::params;
use super::video_config::{
    Headers, Params, RequestFile, RequestFiles, VideoEnvironment, VideoHttpConfig, VideoLogging,
    VideoReply, VideoRequestContext,
};
use crate::api::video_records::VideoObject;

// B249's logging state and the environment its pre-call record is written through
pub struct LogContext<'borrow, 'env> {
    pub state: &'borrow mut LoggingState,
    pub env: &'borrow PrePostEnv<'env>,
}

// The caller's side of LiteLLM's video_generation, with the parameters its handler reads
pub struct VideoGenerationCall<'call> {
    pub model: &'call str,
    pub prompt: &'call str,
    pub optional_params: &'call Params,
    pub custom_llm_provider: &'call str,
    pub litellm_params: &'call Params,
    pub extra_headers: Option<&'call Headers>,
    pub timeout: Duration,
    pub api_key: Option<&'call SecretString>,
}

struct Prepared<'deps, C, S: ClientSource> {
    config: &'deps C,
    client: Selected<'deps, S::Client>,
    headers: Headers,
    api_base: String,
    data: Params,
    files: RequestFiles,
}

struct CreateSend<'send> {
    url: &'send str,
    headers: &'send Headers,
    data: &'send Params,
    files: &'send [RequestFile],
    timeout: Duration,
}

// _is_async hands back a coroutine there; this future is the call itself, and the flag picks the async client and transforms
pub async fn video_generation_handler<C, S>(
    deps: &VideoDeps<'_, C, S>,
    call: &VideoGenerationCall<'_>,
    log: &mut LogContext<'_, '_>,
    is_async: bool,
) -> Result<VideoObject, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
    S: ClientSource,
{
    let kind = if is_async {
        ClientKind::Asynchronous
    } else {
        ClientKind::Sync
    };
    let prepared = prepare(deps, call, kind)?;
    log_pre_call(
        log,
        call.prompt,
        &prepared.api_base,
        &prepared.headers,
        &prepared.data,
    )?;
    let create = CreateSend {
        url: &prepared.api_base,
        headers: &prepared.headers,
        data: &prepared.data,
        files: &prepared.files,
        timeout: call.timeout,
    };
    let sent = Sent {
        mode: kind,
        verb: HttpVerb::Post,
        timeout: Some(call.timeout),
    };
    let response = create_send(prepared.config, prepared.client.get(), &create, sent).await?;
    finish(prepared.config, call, log, &response, &prepared.data)
}

fn prepare<'deps, C, S>(
    deps: &'deps VideoDeps<'deps, C, S>,
    call: &VideoGenerationCall<'_>,
    kind: ClientKind,
) -> Result<Prepared<'deps, C, S>, VideoTransportError<C::Error>>
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
    let api_key = params::api_key(call.api_key, call.litellm_params)?;
    let headers = environment_headers(deps.config, call, api_key.as_ref())?;
    let base = complete_url(deps.config, call)?;
    let (data, files, api_base) = transform_create(deps.config, call, &base, &headers)?;
    Ok(Prepared {
        config: deps.config,
        client,
        headers,
        api_base,
        data,
        files,
    })
}

// validate_environment starts from the optional params' extra_headers, and the call's own headers go over them
fn environment_headers<C>(
    config: &C,
    call: &VideoGenerationCall<'_>,
    api_key: Option<&SecretString>,
) -> Result<Headers, BadField>
where
    C: VideoHttpConfig,
{
    let caller_headers = params::optional_headers(call.optional_params)?;
    let mut headers = config.validate_environment(VideoEnvironment {
        headers: &caller_headers,
        model: call.model,
        api_key,
        litellm_params: Some(call.litellm_params),
    });
    if let Some(extra) = call.extra_headers {
        headers.extend(extra.clone());
    }
    Ok(headers)
}

fn complete_url<C>(
    config: &C,
    call: &VideoGenerationCall<'_>,
) -> Result<String, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
{
    config
        .get_complete_url(
            call.model,
            params::api_base(call.litellm_params)?,
            call.litellm_params,
        )
        .map_err(VideoTransportError::Config)
}

fn transform_create<C>(
    config: &C,
    call: &VideoGenerationCall<'_>,
    api_base: &str,
    headers: &Headers,
) -> Result<(Params, RequestFiles, String), VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
{
    config
        .transform_video_create_request(
            call.model,
            call.prompt,
            call.optional_params,
            VideoRequestContext {
                api_base,
                litellm_params: call.litellm_params,
                headers,
            },
        )
        .map_err(VideoTransportError::Config)
}

// logging_obj.pre_call with an empty api key, since the provider key travels in the headers
fn log_pre_call(
    log: &mut LogContext<'_, '_>,
    prompt: &str,
    api_base: &str,
    headers: &Headers,
    data: &Params,
) -> Result<(), PrePostError> {
    let mut logged = IndexMap::new();
    for (name, value) in headers {
        logged.insert(name.clone(), Json::Text(value.clone()));
    }
    let record = PreCall {
        input: Json::Text(prompt.to_owned()),
        model: None,
        additional_args: AdditionalArgs {
            api_base: Some(api_base.to_owned()),
            headers: Some(logged),
            complete_input_dict: Some(RawBody::Object(data.clone())),
            request_str: None,
            openai_sdk: None,
            passthrough: IndexMap::new(),
        },
    };
    pre_call(log.state, &record, log.env).map(drop)?;
    Ok(())
}

// The three mutually exclusive request paths: files, a file-less multipart form, or JSON, all inside the handler's try
async fn create_send<C, Client>(
    config: &C,
    client: &Client,
    call: &CreateSend<'_>,
    sent: Sent,
) -> Result<UpstreamResponse, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
    Client: VideoTransport,
{
    let body = match create_body(config, call) {
        Ok(body) => body,
        Err(failure) => return exchange(config, client, Err(failure), sent).await,
    };
    let request = build(
        Method::POST,
        call.url,
        call.headers,
        body,
        Some(call.timeout),
    );
    exchange(config, client, request, sent).await
}

fn create_body<'call, C>(
    config: &C,
    call: &CreateSend<'call>,
) -> Result<ServeBody<'call>, SendFailure>
where
    C: VideoHttpConfig,
{
    if !call.files.is_empty() {
        return multipart::with_files(call.data, call.files).map(ServeBody::Parts);
    }
    if config.use_multipart_form_data() {
        return multipart::flattened(call.data).map(ServeBody::Parts);
    }
    Ok(ServeBody::Json(call.data))
}

// The create response is converted after the handler's error path has returned, so its error is never wrapped
fn finish<C>(
    config: &C,
    call: &VideoGenerationCall<'_>,
    log: &mut LogContext<'_, '_>,
    response: &UpstreamResponse,
    data: &Params,
) -> Result<VideoObject, VideoTransportError<C::Error>>
where
    C: VideoHttpConfig,
{
    let mut logging = VideoLogging {
        model_call_details: log.state.model_call_details.clone(),
    };
    let outcome = config.transform_video_create_response(
        call.model,
        VideoReply {
            raw_response: response,
            logging_obj: &mut logging,
            custom_llm_provider: Some(call.custom_llm_provider),
        },
        Some(data),
    );
    // the logging object is live in LiteLLM, so what a failing transform wrote stays for the error path
    log.state.model_call_details = logging.model_call_details;
    outcome.map_err(VideoTransportError::Config)
}
