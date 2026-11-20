use std::sync::Arc;
use std::time::Duration;

use betterlite_config::sections::settings::JsonValue;
use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_http::error::HttpError;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::passthrough_policies::header_forwarding::forward_headers_from_request;
use betterlite_models::resolution::ResolvedModel;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughSigning, PassthroughTarget, PassthroughUrlRequest,
};
use bytes::Bytes;
use futures_util::StreamExt as _;
use futures_util::stream;
use http::HeaderMap;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;
use url::Url;

use super::auth::{Auth, AuthRequest, validate as validate_auth};
use super::config::{ConfigError, Fault, PassthroughCallConfig};
use super::logging::{self, Environment, PreCallRequest, RelayLogging, json_object};
use super::params::{self, CallParams};
use super::wire::{self, WireRequest};
use super::{PassthroughBody, PassthroughCallArgs, PassthroughRelay, PassthroughReply};
use crate::passthrough_stream::async_lifecycle::AsyncPassthroughStream;
use crate::passthrough_stream::async_passthrough_request::{
    AsyncPassthroughReply, FullReply, PassthroughSend, PassthroughSender, async_passthrough_request,
};
use crate::passthrough_stream::error::{PassthroughStreamError, UpstreamStatus};
use crate::passthrough_stream::response::StreamBody;
use crate::passthrough_stream::spend_collection::{OwnedProviderSpend, SpendCollection};

// The upstream's answer once raise_for_status would have raised for it
pub(crate) struct RelayStatus {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Bytes,
}

// Any non-status failure re-resolves its provider, so the call keeps the original exception and the fault
pub(crate) enum Failure {
    Fault(Fault),
    Upstream(Box<RelayStatus>),
}

impl From<ConfigError> for Failure {
    fn from(error: ConfigError) -> Self {
        Self::Fault(Fault::from(error))
    }
}

// What the synthetic stream needs when a redirect answer has to become a relay
struct RelayContext<'call> {
    tag: PassthroughConfigTag,
    model: &'call str,
    provider: &'call str,
    endpoint: &'call str,
    body_limit: usize,
}

struct Upstream {
    url: Url,
    auth: Auth,
}

struct WireSource<'call> {
    args: &'call PassthroughCallArgs,
    url: &'call Url,
    headers: &'call Headers,
    signed: Option<&'call [u8]>,
    json: Option<&'call JsonValue>,
    deadline: Duration,
}

impl<'call> RelayContext<'call> {
    fn new(
        args: &'call PassthroughCallArgs,
        resolved: &'call ResolvedModel,
        tag: PassthroughConfigTag,
        client: &ProxiedClient,
    ) -> Self {
        Self {
            tag,
            model: &resolved.model,
            provider: resolved.provider.slug(),
            endpoint: &args.endpoint,
            body_limit: client.limits.body_bytes,
        }
    }
}

// llm_passthrough_route steps 4 to 15, once the provider and its config are known
pub(crate) async fn run(
    args: &PassthroughCallArgs,
    resolved: &ResolvedModel,
    tag: PassthroughConfigTag,
    state: &mut LoggingState,
    http: &ProxiedClient,
) -> Result<PassthroughReply, Failure> {
    let call = params::build(args, resolved);
    let timeouts = params::timeouts(args, &call.params)?;
    let client = params::client(http, &timeouts)?;
    let config = PassthroughCallConfig::new(tag);
    let request_data = logging_data(args);
    record_environment(state, args, resolved, &call)?;
    let upstream = upstream(&config, args, resolved, &call.params, &client).await?;
    let (headers, signing) = signed_headers(&config, args, resolved, &call.params, &upstream)?;
    let json = replaced_model(args.json.as_ref(), &resolved.model);
    let streaming = is_streaming(&config, args, json.as_ref());
    state.stream = Some(streaming);
    log_pre_call(state, request_data, &upstream.url, &headers)?;
    let request = wire_request(&WireSource {
        args,
        url: &upstream.url,
        headers: &headers,
        signed: signing.as_deref(),
        json: json.as_ref(),
        deadline: timeouts.read,
    })?;
    send(
        request,
        streaming,
        &RelayContext::new(args, resolved, tag, &client),
        &client,
    )
    .await
}

// steps 7 and 8: the complete url, then the provider's own headers and key
async fn upstream(
    config: &PassthroughCallConfig,
    args: &PassthroughCallArgs,
    resolved: &ResolvedModel,
    params: &Object,
    client: &ProxiedClient,
) -> Result<Upstream, Failure> {
    let target = PassthroughTarget {
        model: &resolved.model,
        custom_llm_provider: resolved.provider.slug(),
        endpoint: &args.endpoint,
    };
    let query = args.request_query_params.as_ref().map(query_object);
    let request = PassthroughUrlRequest {
        api_base: resolved.api_base.as_deref(),
        api_key: resolved.api_key.as_ref(),
        target,
        request_query_params: query.as_ref(),
        litellm_params: params,
    };
    let (url, base) = config.get_complete_url(&request)?;
    let url = arn_encoded(url, resolved, &args.endpoint)?;
    let auth = validate_auth(
        config.tag,
        &AuthRequest {
            api_key: resolved.api_key.as_ref(),
            params,
            base: &base,
            http: client,
        },
    )
    .await?;
    Ok(Upstream { url, auth })
}

// steps 9 and 10: the caller's pass-through headers, then the provider's signature over them
fn signed_headers(
    config: &PassthroughCallConfig,
    args: &PassthroughCallArgs,
    resolved: &ResolvedModel,
    params: &Object,
    upstream: &Upstream,
) -> Result<(Headers, Option<Vec<u8>>), Failure> {
    let headers = forward_headers(&args.request_headers, &upstream.auth.headers);
    let signing = PassthroughSigning {
        litellm_params: params,
        request_data: signing_data(args),
        api_base: upstream.url.as_str(),
        model: Some(&resolved.model),
    };
    Ok(config.sign_request(headers, &signing)?)
}

async fn send(
    request: OutboundRequest,
    streaming: bool,
    context: &RelayContext<'_>,
    client: &ProxiedClient,
) -> Result<PassthroughReply, Failure> {
    let relay_logging = Arc::new(RelayLogging);
    let sender: &dyn PassthroughSender = client;
    let relay = PassthroughSend {
        client: sender,
        request,
        is_streaming_request: streaming,
    };
    let reply = async_passthrough_request(
        relay,
        PassthroughCallConfig::new(context.tag),
        Arc::clone(&relay_logging),
        TaskTracker::new(),
    )
    .await;
    match reply {
        Ok(AsyncPassthroughReply::Full(full)) => Ok(full_reply(*full)),
        Ok(AsyncPassthroughReply::Stream(stream)) => stream_reply(stream),
        Err(PassthroughStreamError::UpstreamStatus(status)) => {
            redirected_or_failed(*status, streaming, context, relay_logging)
        }
        Err(PassthroughStreamError::Send(source)) => Err(Failure::Fault(Fault::Send(source))),
        Err(
            error @ (PassthroughStreamError::NotInitialized { .. }
            | PassthroughStreamError::AlreadyAwaited
            | PassthroughStreamError::ExpectedCoroutine
            | PassthroughStreamError::StreamRead(_)
            | PassthroughStreamError::BodyRead(_)),
        ) => Err(Failure::Fault(Fault::message(500, true, error.to_string()))),
    }
}

// raise_for_status fails only 4xx and 5xx, so a redirect is still the answer httpx hands back
fn redirected_or_failed(
    status: UpstreamStatus,
    streaming: bool,
    context: &RelayContext<'_>,
    logging: Arc<RelayLogging>,
) -> Result<PassthroughReply, Failure> {
    let headers = header_pairs(&status.headers);
    if status.status_code >= 400 {
        return Err(Failure::Upstream(Box::new(RelayStatus {
            status: status.status_code,
            headers,
            body: Bytes::from(status.body),
        })));
    }
    if streaming {
        return Ok(synthetic_stream(status, context, logging));
    }
    Ok(PassthroughReply {
        status: status.status_code,
        headers,
        body: PassthroughBody::Full(Box::new(Bytes::from(status.body))),
    })
}

// The relay already read the redirect body, so the stream hands those bytes back as its only chunk
fn synthetic_stream(
    status: UpstreamStatus,
    context: &RelayContext<'_>,
    logging: Arc<RelayLogging>,
) -> PassthroughReply {
    let headers = header_pairs(&status.headers);
    let target = PassthroughTarget {
        model: context.model,
        custom_llm_provider: context.provider,
        endpoint: context.endpoint,
    };
    let stream: PassthroughRelay = AsyncPassthroughStream {
        initialized: true,
        status_code: status.status_code,
        headers: status.headers,
        pending: None,
        source: Some(Box::new(StreamBody {
            chunks: stream::iter(vec![Ok::<Vec<u8>, HttpError>(status.body)]).boxed(),
            body_limit: context.body_limit,
        })),
        spend: SpendCollection {
            sink: Some(OwnedProviderSpend::new(
                PassthroughCallConfig::new(context.tag),
                target,
            )),
            chunk_count: 0,
            failed: false,
        },
        logging,
        background_tasks: TaskTracker::new(),
        flush_scheduled: false,
        hidden_params: IndexMap::new(),
    };
    PassthroughReply {
        status: stream.status_code,
        headers,
        body: PassthroughBody::Stream(Box::new(stream)),
    }
}

fn full_reply(reply: FullReply) -> PassthroughReply {
    PassthroughReply {
        status: reply.status_code,
        headers: header_pairs(&reply.headers),
        body: PassthroughBody::Full(Box::new(Bytes::from(reply.body))),
    }
}

fn stream_reply(stream: Box<PassthroughRelay>) -> Result<PassthroughReply, Failure> {
    let status = stream
        .status_code()
        .map_err(|error| Failure::Fault(Fault::message(500, true, error.to_string())))?;
    let headers = stream
        .headers()
        .map_err(|error| Failure::Fault(Fault::message(500, true, error.to_string())))?;
    Ok(PassthroughReply {
        status,
        headers: header_pairs(headers),
        body: PassthroughBody::Stream(stream),
    })
}

fn record_environment(
    state: &mut LoggingState,
    args: &PassthroughCallArgs,
    resolved: &ResolvedModel,
    call: &CallParams,
) -> Result<(), Failure> {
    let request_data = logging_data(args);
    let environment = Environment {
        model: &resolved.model,
        params: &call.logged,
        endpoint: &args.endpoint,
        provider: resolved.provider.slug(),
        request_data: request_data.as_ref(),
    };
    Ok(logging::record_environment(state, &environment)?)
}

fn log_pre_call(
    state: &mut LoggingState,
    input: Option<Json>,
    url: &Url,
    headers: &Headers,
) -> Result<(), Failure> {
    let request = PreCallRequest {
        input: input.unwrap_or(Json::Null),
        api_base: url.as_str(),
        headers,
    };
    Ok(logging::record_pre_call(state, &request)?)
}

fn wire_request(source: &WireSource<'_>) -> Result<OutboundRequest, Failure> {
    let request = WireRequest {
        method: &source.args.method,
        url: source.url,
        headers: source.headers,
        content: source.args.content.as_ref(),
        signed: source.signed,
        data: signing_data(source.args),
        files: source.args.files.as_deref(),
        json: source.json,
        params: source.args.params.as_deref(),
        cookies: source.args.cookies.as_deref(),
        deadline: source.deadline,
    };
    Ok(wire::build(&request)?)
}

fn is_streaming(
    config: &PassthroughCallConfig,
    args: &PassthroughCallArgs,
    json: Option<&JsonValue>,
) -> bool {
    let empty = Object::new();
    config.is_streaming_request(&args.endpoint, streaming_data(args, json).unwrap_or(&empty))
}

fn query_object(params: &[(String, String)]) -> Object {
    params
        .iter()
        .map(|(name, value)| (name.clone(), JsonValue::String(value.clone())))
        .collect()
}

// llm_passthrough_route encodes an application inference profile id inside the Bedrock runtime url
fn arn_encoded(url: Url, resolved: &ResolvedModel, endpoint: &str) -> Result<Url, ConfigError> {
    if resolved.provider.slug() != "bedrock" || !endpoint.contains("application-inference-profile")
    {
        return Ok(url);
    }
    let encoded = encode_runtime_modelid_arn(url.as_str());
    Url::parse(&encoded).map_err(|error| ConfigError::value_error(error.to_string()))
}

fn forward_headers(request_headers: &Option<Vec<(String, String)>>, auth: &Headers) -> Headers {
    let mut requested = IndexMap::new();
    for (name, value) in request_headers.iter().flatten() {
        requested.insert(name.clone(), value.clone());
    }
    let auth_headers: IndexMap<String, String> = auth
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    forward_headers_from_request(&requested, auth_headers, false)
        .into_iter()
        .collect()
}

// _request_data: data when it is a map at all, then json when it is an object, so an empty map still signs
fn signing_data(args: &PassthroughCallArgs) -> Option<&Object> {
    match &args.data {
        Some(data) => Some(data),
        None => match &args.json {
            Some(JsonValue::Object(fields)) => Some(fields),
            Some(
                JsonValue::Array(_)
                | JsonValue::Bool(_)
                | JsonValue::Null
                | JsonValue::Number(_)
                | JsonValue::String(_),
            )
            | None => None,
        },
    }
}

// request_data for logging and pre_call: data when it is not empty, otherwise json
fn logging_data(args: &PassthroughCallArgs) -> Option<Json> {
    match &args.data {
        Some(data) if !data.is_empty() => Some(Json::Object(json_object(data))),
        _ => args.json.as_ref().map(logging::json),
    }
}

// _streaming_request_data: data when it is a map, else json when it is an object, else nothing
fn streaming_data<'call>(
    args: &'call PassthroughCallArgs,
    json: Option<&'call JsonValue>,
) -> Option<&'call Object> {
    match &args.data {
        Some(data) => Some(data),
        None => match json {
            Some(JsonValue::Object(fields)) => Some(fields),
            Some(
                JsonValue::Array(_)
                | JsonValue::Bool(_)
                | JsonValue::Null
                | JsonValue::Number(_)
                | JsonValue::String(_),
            )
            | None => None,
        },
    }
}

// SWAP MODEL IN JSON BODY: the caller's object reaches the wire with the resolved model inside it
fn replaced_model(json: Option<&JsonValue>, model: &str) -> Option<JsonValue> {
    let mut json = json.cloned()?;
    if let JsonValue::Object(fields) = &mut json {
        if fields.contains_key("model") {
            fields.insert("model".to_owned(), JsonValue::String(model.to_owned()));
        }
    }
    Some(json)
}

fn header_pairs(headers: &HeaderMap) -> Vec<(String, String)> {
    headers
        .iter()
        .map(|(name, value)| (name.as_str().to_owned(), wire::header_text(value)))
        .collect()
}
