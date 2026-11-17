use std::collections::{BTreeMap, VecDeque};

use betterlite_http::proxy::StreamResponse;
use betterlite_models::decisions;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::deployment::ChatDeployment;
use betterlite_openai::chat::execution::{ChatError, ChatExecutor};
use betterlite_openai::stream::SseParser;
use futures_util::StreamExt as _;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use super::error::SdkHttpError;
use super::{ChatMessage, OpenaiChat, native};

pub(crate) const NORMALIZED_ID: &str = "msg_00000000-0000-0000-0000-000000000000";

// What both wrapper drivers send for a case: their call models differ only in how a run is driven
pub(crate) trait WireCall {
    fn model(&self) -> &str;
    fn messages(&self) -> &[ChatMessage];
    fn api_key(&self) -> &str;
    fn timeout_seconds(&self) -> u64;
}

#[derive(Serialize)]
struct WireChat<'wire> {
    model: &'wire str,
    messages: &'wire [ChatMessage],
    stream: bool,
}

pub(crate) struct Prepared {
    pub(crate) executor: ChatExecutor,
    pub(crate) deployment: ChatDeployment,
}

// The same deployment the native chat call builds, so base URLs, keys and timeouts resolve alike
pub(crate) fn prepare<C>(
    call: &C,
    api_base: &str,
    env: &BTreeMap<String, String>,
) -> Result<Prepared, SdkHttpError>
where
    C: WireCall,
{
    let chat = OpenaiChat {
        model: call.model().to_owned(),
        messages: call.messages().to_vec(),
        api_key: call.api_key().to_owned(),
        timeout_seconds: call.timeout_seconds(),
    };
    let executor = native::executor(&chat, api_base, env)?;
    let Some(deployment) = executor.deployments.first().cloned() else {
        return Err(SdkHttpError::Invalid(format!(
            "no deployment serves {}",
            call.model()
        )));
    };
    Ok(Prepared {
        executor,
        deployment,
    })
}

// The body the wrapper sends for a case, parsed the way the native service parses it
pub(crate) fn wire_request<C>(call: &C) -> Result<ChatRequest, SdkHttpError>
where
    C: WireCall + Sync,
{
    let wire = WireChat {
        model: call.model(),
        messages: call.messages(),
        stream: true,
    };
    let body = serde_json::to_vec(&wire).map_err(SdkHttpError::Serialize)?;
    ChatRequest::parse(&body).map_err(|source| SdkHttpError::ChatParse(Box::new(source)))
}

// One POST: the caller decides what a provider answer means to it
pub(crate) async fn open(
    prepared: &Prepared,
    request: &ChatRequest,
) -> Result<StreamResponse, ChatError> {
    prepared
        .executor
        .stream(&prepared.deployment, request)
        .await
}

// The SSE data payloads of one provider answer, in the order the provider sent them
pub(crate) async fn payloads(response: StreamResponse) -> Result<VecDeque<Vec<u8>>, SdkHttpError> {
    let mut stream = response.body_stream();
    let mut parser = SseParser::default();
    let mut payloads = VecDeque::new();
    while let Some(chunk) = stream.next().await {
        let bytes = chunk.map_err(|source| SdkHttpError::Stream(Box::new(source)))?;
        for event in parser
            .push(&bytes)
            .map_err(|error| SdkHttpError::Invalid(error.to_string()))?
        {
            payloads.push_back(event);
        }
    }
    for event in parser.finish() {
        payloads.push_back(event);
    }
    Ok(payloads)
}

// One POST per run, with the body the wrapper would send for the same case
pub(crate) async fn prefetch<C>(
    prepared: &Prepared,
    call: &C,
) -> Result<VecDeque<Vec<u8>>, SdkHttpError>
where
    C: WireCall + Sync,
{
    let request = wire_request(call)?;
    let response = open(prepared, &request)
        .await
        .map_err(|error| SdkHttpError::Chat(Box::new(error)))?;
    payloads(response).await
}

// The uuid is fresh per stream, so both sides replace it with one value before comparing
pub(crate) fn normalized<E>(event: &E, message_id: &str) -> Result<Value, SdkHttpError>
where
    E: Serialize,
{
    let mut value = serde_json::to_value(event).map_err(SdkHttpError::Serialize)?;
    if let Value::Object(fields) = &mut value
        && fields.get("type").and_then(Value::as_str) == Some("message_start")
        && let Some(Value::Object(message)) = fields.get_mut("message")
        && let Some(Value::String(id)) = message.get_mut("id")
        && id.as_str() == message_id
        && id
            .strip_prefix("msg_")
            .is_some_and(|suffix| Uuid::parse_str(suffix).is_ok())
    {
        NORMALIZED_ID.clone_into(id);
    }
    Ok(value)
}

pub(crate) fn seed_object<T>(held: Option<&decisions::Json>) -> Result<Option<T>, SdkHttpError>
where
    T: DeserializeOwned,
{
    let Some(seed) = held else {
        return Ok(None);
    };
    let text = serde_json::to_string(seed).map_err(SdkHttpError::Serialize)?;
    let object = serde_json::from_str(&text).map_err(|source| {
        SdkHttpError::Invalid(format!("an initial held chunk is not an object: {source}"))
    })?;
    Ok(Some(object))
}
