use std::io::Error as IoError;

use betterlite_foundry::chat::environment::{EnvironmentRequest, TokenSource};
use betterlite_foundry::chat::error::ChatError;
use betterlite_foundry::chat::{FoundryChatOptions, PreparedChat as LeafPreparedChat};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_prompts::message_content::Message;
use http::header::{CONTENT_TYPE, HeaderName, HeaderValue, InvalidHeaderName, InvalidHeaderValue};
use http::{HeaderMap, Method};
use indexmap::IndexMap;
use secrecy::SecretString;
use url::Url;

use super::super::command_r::{self, CommandRError};
use super::super::credentials::{Globals, headers_or_global, resolve_key};
use super::super::route::{self, Route};
use super::{Call, EXTRA_HEADERS_FIELD};
use crate::operation::OperationError;
use crate::prepared::body::chat_request_fields;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest};
use crate::transform_retry::TransformRetry;

const API_VERSION_FIELD: &str = "api_version";
const JSON_CONTENT_TYPE: &str = "application/json";

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("the {operation} branch of the Foundry provider is not this chat call")]
    Route { operation: &'static str },
    #[error("the {name} header name is not one HTTP can carry: {source}")]
    HeaderName {
        name: String,
        #[source]
        source: InvalidHeaderName,
    },
    #[error("the {name} header holds a value HTTP cannot carry: {source}")]
    HeaderValue {
        name: String,
        #[source]
        source: InvalidHeaderValue,
    },
    #[error("the Azure Foundry request could not be written as JSON: {source}")]
    Encode {
        #[from]
        source: serde_json::Error,
    },
    #[error(transparent)]
    CommandR(#[from] CommandRError),
    #[error(transparent)]
    Chat(#[from] ChatError),
    #[error(transparent)]
    Request(#[from] OperationError),
}

impl From<PrepareError> for OperationError {
    fn from(error: PrepareError) -> Self {
        match error {
            PrepareError::Encode { source } => Self::Encode { source },
            PrepareError::HeaderValue { name, source } => Self::InvalidHeader { name, source },
            PrepareError::Request(source) => source,
            other @ (PrepareError::Route { .. }
            | PrepareError::HeaderName { .. }
            | PrepareError::CommandR(_)
            | PrepareError::Chat(_)) => Self::UnsupportedOperation {
                provider: String::from("azure_ai"),
                operation: other.to_string(),
            },
        }
    }
}

// The prepared request plus what main.py resolved and logged around it
pub struct Plan {
    pub prepared: PreparedRequest,
    pub messages: Vec<Message>,
    pub headers: Option<Headers>,
    pub base: Option<String>,
    pub key: Option<SecretString>,
}

// The messages, optional parameters and api_version main.py hands the Azure AI config
struct Wire {
    messages: Vec<Message>,
    optional: Object,
    api_version: Option<String>,
}

// What main.py resolved from the call and the globals before the config runs
struct Resolved {
    base: Option<String>,
    key: Option<SecretString>,
    headers: Option<Headers>,
}

pub fn prepare(call: &Call<'_>, globals: &Globals<'_>) -> Result<Plan, PrepareError> {
    require_chat(call.model)?;
    let (wire, extra_headers) = wire_parts(call)?;
    let resolved = Resolved {
        base: globals
            .accessors()
            .api_base(call.api_base.filter(|text| !text.is_empty())),
        key: resolve_key(call.api_key, globals),
        headers: headers_or_global(call.headers, globals).cloned(),
    };
    let leaf = leaf_chat(call, globals, &resolved, &wire)?;
    let chat = assembled(&leaf, call, extra_headers.as_ref())?;
    let prepared = if call.stream {
        PreparedRequest::Stream(Box::new(chat))
    } else {
        PreparedRequest::Chat(Box::new(chat))
    };
    Ok(Plan {
        prepared,
        messages: wire.messages,
        headers: resolved.headers,
        base: resolved.base,
        key: resolved.key,
    })
}

fn require_chat(model: &str) -> Result<(), PrepareError> {
    match route::select(model) {
        Route::Chat => Ok(()),
        Route::Agents => Err(PrepareError::Route {
            operation: "agents",
        }),
        Route::Claude => Err(PrepareError::Route {
            operation: "claude",
        }),
    }
}

fn wire_parts(call: &Call<'_>) -> Result<(Wire, Option<Headers>), PrepareError> {
    let fields = chat_request_fields(call.request)?;
    let sent: Vec<Message> = serde_json::to_value(fields.messages)
        .and_then(serde_json::from_value)
        .map_err(|source| PrepareError::Encode { source })?;
    let messages = command_r::transform(&sent, call.model)?;
    let mut params = fields.params;
    if let Some(extra) = call.extra_headers {
        params.insert(
            String::from(EXTRA_HEADERS_FIELD),
            JsonValue::Object(
                extra
                    .iter()
                    .map(|(name, value)| (name.clone(), JsonValue::Text(value.clone())))
                    .collect(),
            ),
        );
    }
    let extra_headers = take_extra_headers(&mut params)?;
    let api_version = text_field(params.shift_remove(API_VERSION_FIELD));
    let optional = object(JsonValue::Object(params))?;
    let parts = Wire {
        messages,
        optional,
        api_version,
    };
    Ok((parts, extra_headers))
}

fn leaf_chat(
    call: &Call<'_>,
    globals: &Globals<'_>,
    resolved: &Resolved,
    wire: &Wire,
) -> Result<LeafPreparedChat, ChatError> {
    FoundryChatOptions {
        model: call.model,
        api_version: wire.api_version.as_deref(),
        messages: &wire.messages,
        optional_params: &wire.optional,
        environment: EnvironmentRequest {
            headers: resolved.headers.clone().unwrap_or_default(),
            argument_key: resolved.key.as_ref(),
            params_key: None,
            configured_key: None,
            token: TokenSource {
                credentials: globals.credentials,
                refresh_enabled: globals.refresh_enabled,
                provider: globals.token_provider.clone(),
                sources: globals.sources,
            },
            api_base: resolved.base.as_deref(),
        },
    }
    .prepare()
}

// main.py leaves extra_headers in optional_params and the OpenAI-compatible handler moves them onto the wire
fn take_extra_headers(
    fields: &mut IndexMap<String, JsonValue>,
) -> Result<Option<Headers>, PrepareError> {
    match fields.shift_remove(EXTRA_HEADERS_FIELD) {
        Some(JsonValue::Object(pairs)) => pairs
            .into_iter()
            .map(|(name, value)| match value {
                JsonValue::Text(text) => Ok((name, text)),
                JsonValue::Null
                | JsonValue::Bool(_)
                | JsonValue::Number(_)
                | JsonValue::Array(_)
                | JsonValue::Object(_) => Err(PrepareError::Encode {
                    source: serde_json::Error::io(IoError::other(
                        "an extra header value is not a string",
                    )),
                }),
            })
            .collect::<Result<Headers, PrepareError>>()
            .map(Some),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Text(_),
        ) => Err(PrepareError::Encode {
            source: serde_json::Error::io(IoError::other("extra_headers is not an object")),
        }),
        None => Ok(None),
    }
}

fn assembled(
    leaf: &LeafPreparedChat,
    call: &Call<'_>,
    extra: Option<&Headers>,
) -> Result<PreparedChat, PrepareError> {
    let url = Url::parse(&leaf.url).map_err(|source| PrepareError::Encode {
        source: serde_json::Error::io(IoError::other(source)),
    })?;
    let body = serde_json::to_vec(&leaf.body)?;
    let http = PreparedHttp {
        method: Method::POST,
        url,
        headers: header_map(&leaf.headers, extra)?,
        body,
        deadline: call.deployment.timeout,
        body_limit: None,
    };
    Ok(PreparedChat {
        http,
        retry: retry()?,
        client_model: Some(call.model.to_owned()),
    })
}

fn header_map(headers: &Headers, extra: Option<&Headers>) -> Result<HeaderMap, PrepareError> {
    let mut map = HeaderMap::new();
    for (name, value) in headers.iter().chain(extra.into_iter().flatten()) {
        let header_name =
            HeaderName::try_from(name.as_str()).map_err(|source| PrepareError::HeaderName {
                name: name.clone(),
                source,
            })?;
        let mut header_value =
            HeaderValue::try_from(value.as_str()).map_err(|source| PrepareError::HeaderValue {
                name: name.clone(),
                source,
            })?;
        header_value.set_sensitive(true);
        map.insert(header_name, header_value);
    }
    map.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
    Ok(map)
}

// AzureAIStudioConfig.max_retry_on_unprocessable_entity_error is 2 whatever the drop policy says
fn retry() -> Result<TransformRetry, PrepareError> {
    TransformRetry::bounded(2).map_err(|source| PrepareError::Encode {
        source: serde_json::Error::io(IoError::other(source)),
    })
}

fn object(value: JsonValue) -> Result<Object, PrepareError> {
    match value {
        JsonValue::Object(fields) => serde_json::to_value(fields)
            .and_then(serde_json::from_value)
            .map_err(|source| PrepareError::Encode { source }),
        JsonValue::Null
        | JsonValue::Bool(_)
        | JsonValue::Number(_)
        | JsonValue::Text(_)
        | JsonValue::Array(_) => Err(PrepareError::Encode {
            source: serde_json::Error::io(IoError::other(
                "the optional parameters are not an object",
            )),
        }),
    }
}

fn text_field(value: Option<JsonValue>) -> Option<String> {
    match value {
        Some(JsonValue::Text(text)) => Some(text),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    }
}
