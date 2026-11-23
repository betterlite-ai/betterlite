use std::time::Duration;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::decisions::Json;
use betterlite_openai::files::error::StatusError;
use betterlite_openai::files::stream::flatten_headers;
use http::Method;
use http::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderName, HeaderValue};
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};

use super::client::{
    ClientFamily, ClientWiring, RequestAuth, SelectedSpeechClient, SpeechConnection, request_auth,
    select_client, v1_auth,
};
use super::error::AzureSpeechError;
use super::logging::{SpeechLogger, SpeechPreCall};
use super::url::compose_url;
use crate::endpoints::projection::{CallableProvider, LiteLlmParams, SessionKind, SuppliedClient};
use crate::native_speech::response::HttpxBinaryResponseContent;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const SPEECH_PATH: &str = "audio/speech";
const RESERVED: [&str; 3] = ["input", "model", "voice"];

// AzureChatCompletion.audio_speech and async_audio_speech take these arguments; only litellm_params reaches the client builder.
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

struct SpeechControls {
    headers: Vec<(String, String)>,
    query: Vec<(String, String)>,
    body: IndexMap<String, Json>,
    timeout: Option<Duration>,
}

// One binary speech post: pop the call controls, select the SDK client, log the pre-call, then build the wire request
pub fn audio_speech(
    call: &AudioSpeechCall<'_>,
    wiring: &mut ClientWiring<'_, '_>,
    logger: &mut dyn SpeechLogger,
) -> Result<OutboundRequest, AzureSpeechError> {
    let mut options = call.optional_params.clone();
    options.shift_remove("max_retries");
    let logged = logged_body(call.model, call.voice, &options);
    let controls = SpeechControls::take(&mut options)?;
    let connection = SpeechConnection {
        model: call.model.to_owned(),
        api_key: call.api_key.clone(),
        api_base: call.api_base.clone(),
        api_version: call.api_version.clone(),
        params: call.params.clone(),
        provider: call.provider.clone(),
        supplied: call.supplied.clone(),
        session: call.session,
        refresh_enabled: call.refresh_enabled,
    };
    let selected = select_client(&connection, call.aspeech, wiring)?;
    logger
        .pre_call(SpeechPreCall {
            input: call.input,
            api_base: &selected.base_url,
            data: logged,
        })
        .map_err(|source| AzureSpeechError::Logging { source })?;
    let auth = auth_for(&selected, &controls.headers)?;
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/octet-stream"));
    for (name, value) in &controls.headers {
        headers.insert(header_name(name)?, header_value(name, value)?);
    }
    insert_auth(&mut headers, auth.as_ref())?;
    let (deployment, version) = match selected.family {
        ClientFamily::Azure => (Some(call.model), selected.api_version.as_deref()),
        ClientFamily::OpenAiV1 => (None, None),
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
        deadline: deadline(call, controls.timeout)?,
    })
}

// HttpxBinaryResponseContent(response=response.response): a non-2xx reply is the SDK's status error
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

impl SpeechControls {
    fn take(options: &mut IndexMap<String, Json>) -> Result<Self, AzureSpeechError> {
        Ok(Self {
            headers: string_pairs("extra_headers", options.shift_remove("extra_headers"))?,
            query: string_pairs("extra_query", options.shift_remove("extra_query"))?,
            body: match options.shift_remove("extra_body") {
                None | Some(Json::Null) => IndexMap::new(),
                Some(Json::Object(fields)) => fields,
                Some(other) => {
                    return Err(AzureSpeechError::ExtraBody {
                        given: python_type(&other).to_owned(),
                    });
                }
            },
            timeout: match options.shift_remove("timeout") {
                None | Some(Json::Null) => None,
                Some(Json::Number(number)) => {
                    Some(duration_of(number.as_f64().ok_or_else(timeout_error)?)?)
                }
                Some(Json::Bool(_) | Json::Text(_) | Json::List(_) | Json::Object(_)) => {
                    return Err(timeout_error());
                }
            },
        })
    }
}

// speech_request_body: model and voice, then every option except the caller headers
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
    for (name, value) in fields {
        let Json::Text(text) = value else {
            return Err(AzureSpeechError::Option {
                option: option.to_owned(),
                problem: "holds a value that is not a string",
            });
        };
        pairs.push((name, text));
    }
    Ok(pairs)
}

fn python_type(value: &Json) -> &'static str {
    match value {
        Json::Null => "<class 'NoneType'>",
        Json::Bool(_) => "<class 'bool'>",
        Json::Number(_) => "<class 'int'>",
        Json::Text(_) => "<class 'str'>",
        Json::List(_) => "<class 'list'>",
        Json::Object(_) => "<class 'dict'>",
    }
}

fn timeout_error() -> AzureSpeechError {
    AzureSpeechError::Option {
        option: String::from("timeout"),
        problem: "is not a number of seconds",
    }
}

fn duration_of(seconds: f64) -> Result<Duration, AzureSpeechError> {
    Duration::try_from_secs_f64(seconds).map_err(|_invalid| timeout_error())
}

// the method's timeout argument never reaches SDK construction; litellm_params' timeout does
fn deadline(
    call: &AudioSpeechCall<'_>,
    request_timeout: Option<Duration>,
) -> Result<Duration, AzureSpeechError> {
    if let Some(timeout) = request_timeout {
        return Ok(timeout);
    }
    call.params.timeout.map_or(Ok(DEFAULT_TIMEOUT), duration_of)
}

// ad_token first, then the key; a caller header the SDK would keep lets a missing credential through
fn auth_for(
    selected: &SelectedSpeechClient<'_>,
    extra_headers: &[(String, String)],
) -> Result<Option<RequestAuth>, AzureSpeechError> {
    match selected.family {
        ClientFamily::OpenAiV1 => Ok(Some(v1_auth(selected.v1_key.as_ref())?)),
        ClientFamily::Azure => match request_auth(
            selected.ad_token.clone(),
            selected.provider.as_ref(),
            selected.api_key.clone(),
        ) {
            Ok(auth) => Ok(Some(auth)),
            Err(AzureSpeechError::UnableToHandleAuth) => {
                if has_auth_header(extra_headers) {
                    Ok(None)
                } else {
                    Err(AzureSpeechError::UnableToHandleAuth)
                }
            }
            Err(error) => Err(error),
        },
    }
}

fn has_auth_header(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(name, _)| {
        name.eq_ignore_ascii_case("authorization") || name.eq_ignore_ascii_case("api-key")
    })
}

fn insert_auth(
    headers: &mut HeaderMap,
    auth: Option<&RequestAuth>,
) -> Result<(), AzureSpeechError> {
    match auth {
        Some(RequestAuth::ApiKey(key)) => {
            let name = HeaderName::from_static("api-key");
            if !headers.contains_key(&name) {
                headers.insert(name, sensitive_value("api-key", key.expose_secret())?);
            }
        }
        Some(RequestAuth::Bearer(token)) => {
            if !headers.contains_key(AUTHORIZATION) {
                let value = format!("Bearer {}", token.expose_secret());
                headers.insert(AUTHORIZATION, sensitive_value("authorization", &value)?);
            }
        }
        None => {}
    }
    Ok(())
}

fn header_name(name: &str) -> Result<HeaderName, AzureSpeechError> {
    HeaderName::from_bytes(name.as_bytes()).map_err(|source| AzureSpeechError::HeaderName {
        name: name.to_owned(),
        source,
    })
}

fn header_value(name: &str, value: &str) -> Result<HeaderValue, AzureSpeechError> {
    HeaderValue::try_from(value).map_err(|source| AzureSpeechError::HeaderValue {
        name: name.to_owned(),
        source,
    })
}

fn sensitive_value(name: &str, value: &str) -> Result<HeaderValue, AzureSpeechError> {
    let mut parsed = header_value(name, value)?;
    parsed.set_sensitive(true);
    Ok(parsed)
}

fn wire_body(
    call: &AudioSpeechCall<'_>,
    options: IndexMap<String, Json>,
    extra_body: IndexMap<String, Json>,
) -> Result<Vec<u8>, AzureSpeechError> {
    let mut body = IndexMap::from([
        (String::from("input"), Json::Text(call.input.to_owned())),
        (String::from("model"), Json::Text(call.model.to_owned())),
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
