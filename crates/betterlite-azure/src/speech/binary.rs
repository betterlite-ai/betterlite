use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::files::error::StatusError;
use betterlite_openai::files::stream::flatten_headers;
use http::Method;
use indexmap::IndexMap;
use secrecy::SecretString;

use super::client::{ClientWiring, Family, SpeechConnection, request_auth, select_client};
use super::error::AzureSpeechError;
use super::logging::{SpeechLogger, SpeechPreCall};
use super::wire::{binary_headers, compose_url};
use crate::endpoints::projection::{CallableProvider, LiteLlmParams, SessionKind, SuppliedClient};
use crate::native_speech::response::HttpxBinaryResponseContent;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const SPEECH_PATH: &str = "audio/speech";
const RESERVED: [&str; 3] = ["input", "model", "voice"];

// AzureChatCompletion.audio_speech takes these arguments; only litellm_params reaches the client builder.
pub struct AudioSpeechCall<'data> {
    pub model: &'data str,
    pub input: &'data str,
    pub voice: &'data str,
    pub optional_params: IndexMap<String, Json>,
    pub aspeech: bool,
    pub api_key: Option<SecretString>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    pub params: LiteLlmParams,
    pub provider: Option<CallableProvider>,
    pub supplied: Option<SuppliedClient>,
    pub session: SessionKind,
    pub refresh_enabled: bool,
}

// The keys speech.create takes as SDK options rather than body fields, and the request timeout.
struct Controls {
    headers: Vec<(String, String)>,
    query: Vec<(String, String)>,
    body: IndexMap<String, Json>,
    timeout: Option<Duration>,
}

// One binary speech post: pop the call controls, select the SDK client, log the pre-call, then build the wire request.
pub fn audio_speech(
    call: &AudioSpeechCall<'_>,
    wiring: &mut ClientWiring<'_, '_>,
    logger: &mut dyn SpeechLogger,
) -> Result<OutboundRequest, AzureSpeechError> {
    let mut options = call.optional_params.clone();
    options.shift_remove("max_retries");
    let complete_input = logged_body(call.model, call.voice, &options);
    let controls = Controls::take(&mut options)?;
    let selected = select_client(&connection(call), wiring)?;
    logger.pre_call(SpeechPreCall {
        input: call.input,
        api_base: &selected.base_url,
        data: complete_input,
    })?;
    let caller_auth = controls.headers.iter().any(|(name, _)| {
        name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("api-key")
    });
    let auth = request_auth(&selected, caller_auth)?;
    let headers = binary_headers(&controls.headers, auth.as_ref())?;
    let (deployment, version) = match selected.family {
        Family::Azure => (Some(call.model), selected.api_version.as_deref()),
        Family::OpenAiV1 => (None, None),
    };
    let url = compose_url(
        &selected.base_url,
        deployment,
        SPEECH_PATH,
        version,
        &controls.query,
    )?;
    Ok(OutboundRequest {
        method: Method::POST,
        url,
        query: Vec::new(),
        headers,
        body: RequestBody::Json(wire_body(call, options, controls.body)?),
        deadline: deadline(&call.params, controls.timeout)?,
    })
}

// The SDK construction arguments, which are exactly the call's own and none of its speech controls.
fn connection(call: &AudioSpeechCall<'_>) -> SpeechConnection {
    SpeechConnection {
        model: call.model.to_owned(),
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        api_version: call.api_version.clone(),
        params: call.params.clone(),
        provider: call.provider.clone(),
        supplied: call.supplied.clone(),
        session: call.session,
        refresh_enabled: call.refresh_enabled,
        aspeech: call.aspeech,
    }
}

// HttpxBinaryResponseContent(response=response.response): a non-2xx reply is the SDK's status error.
pub fn finish_audio_speech(
    reply: UpstreamResponse,
) -> Result<HttpxBinaryResponseContent, AzureSpeechError> {
    if !(200..300).contains(&reply.status) {
        return Err(AzureSpeechError::Status(Box::new(
            StatusError::from_response(reply.status, &reply.body),
        )));
    }
    Ok(HttpxBinaryResponseContent {
        status: reply.status,
        headers: flatten_headers(&reply.headers).into_iter().collect(),
        body: reply.body,
    })
}

impl Controls {
    fn take(options: &mut IndexMap<String, Json>) -> Result<Self, AzureSpeechError> {
        Ok(Self {
            headers: string_pairs("extra_headers", options.shift_remove("extra_headers"))?,
            query: string_pairs("extra_query", options.shift_remove("extra_query"))?,
            body: match options.shift_remove("extra_body") {
                None | Some(Json::Null) => IndexMap::new(),
                Some(Json::Object(fields)) => fields,
                Some(other) => {
                    return Err(AzureSpeechError::ExtraBody {
                        given: json_kind(&other),
                    });
                }
            },
            timeout: match options.shift_remove("timeout") {
                None | Some(Json::Null) => None,
                Some(Json::Number(number)) => Some(seconds_duration(
                    number.as_f64().ok_or_else(timeout_error)?,
                )?),
                Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
                    return Err(timeout_error());
                }
            },
        })
    }
}

// speech_request_body: model and voice, then every option except the caller headers the SDK takes as kwargs.
fn logged_body(
    model: &str,
    voice: &str,
    options: &IndexMap<String, Json>,
) -> IndexMap<String, Json> {
    let mut body = IndexMap::from([
        (String::from("model"), Json::Text(model.to_owned())),
        (String::from("voice"), Json::Text(voice.to_owned())),
    ]);
    for (name, value) in options {
        if name != "extra_headers" {
            body.insert(name.clone(), value.clone());
        }
    }
    body
}

fn string_pairs(
    option: &str,
    value: Option<Json>,
) -> Result<Vec<(String, String)>, AzureSpeechError> {
    let fields = match value {
        None | Some(Json::Null) => return Ok(Vec::new()),
        Some(Json::Object(fields)) => fields,
        Some(Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_)) => {
            return Err(AzureSpeechError::Option {
                option: option.to_owned(),
                problem: "is not a mapping of strings",
            });
        }
    };
    let mut pairs = Vec::with_capacity(fields.len());
    for (name, held) in fields {
        let Json::Text(text) = held else {
            return Err(AzureSpeechError::Option {
                option: option.to_owned(),
                problem: "holds a value that is not a string",
            });
        };
        pairs.push((name, text));
    }
    Ok(pairs)
}

const fn json_kind(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "bool",
        Json::Number(_) => "number",
        Json::Text(_) => "string",
        Json::List(_) => "array",
        Json::Object(_) => "object",
    }
}

fn timeout_error() -> AzureSpeechError {
    AzureSpeechError::Option {
        option: String::from("timeout"),
        problem: "is not a number of seconds",
    }
}

fn seconds_duration(seconds: f64) -> Result<Duration, AzureSpeechError> {
    match Duration::try_from_secs_f64(seconds) {
        Ok(duration) => Ok(duration),
        Err(_invalid) => Err(timeout_error()),
    }
}

// the method's timeout argument never reaches SDK construction; litellm_params' timeout does
fn deadline(
    params: &LiteLlmParams,
    request_timeout: Option<Duration>,
) -> Result<Duration, AzureSpeechError> {
    if let Some(timeout) = request_timeout {
        return Ok(timeout);
    }
    params
        .timeout
        .map_or_else(|| Ok(DEFAULT_TIMEOUT), seconds_duration)
}

fn wire_body(
    call: &AudioSpeechCall<'_>,
    options: IndexMap<String, Json>,
    extra_body: IndexMap<String, Json>,
) -> Result<Vec<u8>, AzureSpeechError> {
    let mut body = IndexMap::from([
        (String::from("model"), Json::Text(call.model.to_owned())),
        (String::from("input"), Json::Text(call.input.to_owned())),
        (String::from("voice"), Json::Text(call.voice.to_owned())),
    ]);
    for (name, value) in options {
        if let Some(argument) = RESERVED
            .iter()
            .find(|argument| **argument == name.as_str())
            .copied()
        {
            return Err(AzureSpeechError::DuplicateArgument { argument });
        }
        body.insert(name, value);
    }
    for (name, value) in extra_body {
        body.insert(name, value);
    }
    serde_json::to_vec(&body).map_err(AzureSpeechError::Encode)
}
