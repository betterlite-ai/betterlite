use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::settings::JsonValue;
use betterlite_context::bedrock_model::runtime_arn::encode_runtime_modelid_arn;
use betterlite_context::resource_config_selection::passthrough::PassthroughConfigTag;
use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_logging::logging_state::LoggingState;
use betterlite_models::decisions::Json;
use betterlite_models::passthrough_policies::header_forwarding::forward_headers_from_request;
use betterlite_models::resolution::ResolvedModel;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::passthrough::transformation::BasePassthroughConfig as _;
use betterlite_openai::passthrough::transformation::types::{
    PassthroughSigning, PassthroughTarget, PassthroughUrlRequest,
};
use bytes::Bytes;
use indexmap::IndexMap;
use tokio_util::task::TaskTracker;
use url::Url;

use super::auth::validate as validate_auth;
use super::aws::{AwsFlavor, AwsPassthrough};
use super::config::{ConfigError, Fault, FaultDetail, PassthroughCallConfig};
use super::logging::{self, RelayLogging};
use super::params;
use super::wire::{self, WireRequest};
use super::{PassthroughCallArgs, PassthroughReply};
use crate::passthrough_stream::async_passthrough_request::{
    AsyncPassthroughReply, PassthroughSend, PassthroughSender, async_passthrough_request,
};
use crate::passthrough_stream::error::PassthroughStreamError;

mod reply;

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

// What the reply conversion needs when a redirect answer has to become a relay
pub(crate) struct RelayContext<'call> {
    pub tag: PassthroughConfigTag,
    pub environment: Arc<BTreeMap<String, String>>,
    pub aws: Arc<dyn AwsPassthrough>,
    pub model: &'call str,
    pub provider: &'call str,
    pub endpoint: &'call str,
    pub body_limit: usize,
}

struct Upstream {
    url: Url,
    headers: Headers,
}

// llm_passthrough_route steps 4 to 15, once the provider and its config are known
pub(crate) async fn run(
    args: &PassthroughCallArgs,
    resolved: &ResolvedModel,
    config: &PassthroughCallConfig,
    state: &mut LoggingState,
    http: &ProxiedClient,
) -> Result<PassthroughReply, Failure> {
    let call = params::build(args, resolved);
    let timeouts = params::timeouts(args, &call.params)?;
    let client = params::client(http, &timeouts)?;
    logging::record_environment(state, args, resolved, &call)?;
    let upstream = upstream(config, args, resolved, &call.params, &client).await?;
    let signing = PassthroughSigning {
        litellm_params: &call.params,
        request_data: object_data(args.data.as_ref(), args.json.as_ref()),
        api_base: upstream.url.as_str(),
        model: Some(&resolved.model),
    };
    let request_headers = args.request_headers.as_deref().unwrap_or_default();
    let (headers, signed) = signed_headers(config, request_headers, &upstream, &signing).await?;
    let json = replaced_model(args.json.as_ref(), &resolved.model);
    let empty = Object::new();
    let streaming = config.is_streaming_request(
        &args.endpoint,
        object_data(args.data.as_ref(), json.as_ref()).unwrap_or(&empty),
    );
    state.stream = Some(streaming);
    let input = logging::request_data(args).unwrap_or(Json::Null);
    logging::record_pre_call(state, &input, upstream.url.as_str(), &headers)?;
    let request = WireRequest {
        method: &args.method,
        url: &upstream.url,
        headers: &headers,
        content: args.content.as_ref(),
        signed: signed.as_deref(),
        data: args.data.as_ref(),
        files: args.files.as_deref(),
        json: json.as_ref(),
        params: args.params.as_deref(),
        cookies: args.cookies.as_deref(),
        deadline: timeouts.read,
    };
    let context = relay_context(config, args, resolved, &client);
    send(wire::build(&request)?, streaming, &context, &client).await
}

// what the reply conversion needs from the call: the target a redirect relay reports
fn relay_context<'call>(
    config: &PassthroughCallConfig,
    args: &'call PassthroughCallArgs,
    resolved: &'call ResolvedModel,
    client: &ProxiedClient,
) -> RelayContext<'call> {
    RelayContext {
        tag: config.tag,
        environment: Arc::clone(&config.environment),
        aws: Arc::clone(&config.aws),
        model: &resolved.model,
        provider: resolved.provider.slug(),
        endpoint: &args.endpoint,
        body_limit: client.limits.body_bytes,
    }
}

// steps 7 and 8: the complete url, the ARN encoding, then the provider's own headers
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
    let query = args.request_query_params.as_ref().map(|pairs| {
        pairs
            .iter()
            .map(|(name, value)| (name.clone(), JsonValue::String(value.clone())))
            .collect::<Object>()
    });
    let request = PassthroughUrlRequest {
        api_base: resolved.api_base.as_deref(),
        api_key: resolved.api_key.as_ref(),
        target,
        request_query_params: query.as_ref(),
        litellm_params: params,
    };
    let (url, base) = config.get_complete_url(&request)?;
    let encoded = arn_encoded(url, resolved, &args.endpoint)?;
    let headers = validate_auth(config, resolved.api_key.as_ref(), params, &base, client).await?;
    Ok(Upstream {
        url: encoded,
        headers,
    })
}

// steps 9 and 10: the caller's pass-through headers, then the provider's signature over them
async fn signed_headers(
    config: &PassthroughCallConfig,
    request_headers: &[(String, String)],
    upstream: &Upstream,
    signing: &PassthroughSigning<'_>,
) -> Result<(Headers, Option<Vec<u8>>), Failure> {
    let requested: IndexMap<String, String> = request_headers.iter().cloned().collect();
    let auth_headers: IndexMap<String, String> = upstream
        .headers
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    let headers = forward_headers_from_request(&requested, auth_headers, false)
        .into_iter()
        .collect();
    let aws_flavor = match config.tag {
        PassthroughConfigTag::Bedrock => Some(AwsFlavor::Bedrock),
        PassthroughConfigTag::BedrockMantle => Some(AwsFlavor::Mantle),
        PassthroughConfigTag::VLLM
        | PassthroughConfigTag::Azure
        | PassthroughConfigTag::AzureAI
        | PassthroughConfigTag::GigaChat
        | PassthroughConfigTag::Watsonx
        | PassthroughConfigTag::NvidiaNim => None,
    };
    let Some(flavor) = aws_flavor else {
        return Ok(config.sign_request(headers, signing)?);
    };
    Ok(config.aws.sign_request(flavor, headers, signing).await?)
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
        PassthroughCallConfig {
            tag: context.tag,
            environment: Arc::clone(&context.environment),
            aws: Arc::clone(&context.aws),
        },
        Arc::clone(&relay_logging),
        TaskTracker::new(),
    )
    .await;
    match reply {
        Ok(AsyncPassthroughReply::Full(full)) => Ok(reply::full(*full)),
        Ok(AsyncPassthroughReply::Stream(stream)) => reply::stream(stream),
        Err(PassthroughStreamError::UpstreamStatus(status)) => {
            reply::answer(*status, streaming, context, relay_logging)
        }
        Err(PassthroughStreamError::Send(source)) => Err(Failure::Fault(Fault::Send(source))),
        Err(
            error @ (PassthroughStreamError::NotInitialized { .. }
            | PassthroughStreamError::AlreadyAwaited
            | PassthroughStreamError::ExpectedCoroutine
            | PassthroughStreamError::StreamRead(_)
            | PassthroughStreamError::BodyRead(_)),
        ) => Err(relay_fault(&error)),
    }
}

// The failures B3572 only names, such as a second read of the same stream, reach _handle_error as a 500
pub(super) fn relay_fault(error: &PassthroughStreamError) -> Failure {
    Failure::Fault(Fault::Failure(Box::new(FaultDetail {
        status: 500,
        synthesized: true,
        message: error.to_string(),
    })))
}

// llm_passthrough_route encodes an application inference profile id inside the Bedrock runtime url
pub(super) fn arn_encoded(
    url: Url,
    resolved: &ResolvedModel,
    endpoint: &str,
) -> Result<Url, ConfigError> {
    if resolved.provider.slug() != "bedrock" || !endpoint.contains("application-inference-profile")
    {
        return Ok(url);
    }
    let encoded = encode_runtime_modelid_arn(url.as_str());
    Url::parse(&encoded).map_err(|error| ConfigError::ValueError {
        message: error.to_string(),
    })
}

// _request_data: data when it is a map at all, then json when it is an object, so an empty map still signs
const fn object_data<'value>(
    data: Option<&'value Object>,
    json: Option<&'value JsonValue>,
) -> Option<&'value Object> {
    if let Some(map) = data {
        return Some(map);
    }
    match json {
        Some(JsonValue::Object(fields)) => Some(fields),
        Some(
            JsonValue::Array(_)
            | JsonValue::Bool(_)
            | JsonValue::Null
            | JsonValue::Number(_)
            | JsonValue::String(_),
        )
        | None => None,
    }
}

// SWAP MODEL IN JSON BODY: the caller's object reaches the wire with the resolved model inside it
fn replaced_model(json: Option<&JsonValue>, model: &str) -> Option<JsonValue> {
    let mut body = json.cloned()?;
    if let JsonValue::Object(fields) = &mut body
        && fields.contains_key("model")
    {
        fields.insert("model".to_owned(), JsonValue::String(model.to_owned()));
    }
    Some(body)
}
