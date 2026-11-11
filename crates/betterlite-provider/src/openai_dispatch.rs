use std::io::Error as IoError;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::ChatRequest;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers as OpenAiHeaders, OpenAiConfig, OpenAiSettings,
};
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use indexmap::IndexMap;
use secrecy::ExposeSecret as _;
use serde::Serialize;
use serde_json::Error as JsonError;

use crate::operation::{
    ChatCompletion, ChatOperation, EmbeddingOperation, OperationError, OperationRequest,
    OperationResult, success,
};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedRequest, Upstream};
use crate::transform_retry::{TransformOnUnprocessable, TransformRetry};

// LiteLLM's DEFAULT_CHAT_COMPLETION_PARAM_VALUES: these go through the OpenAI parameter policy, the rest are forwarded
const KNOWN_PARAMS: &str = "functions function_call temperature top_p n stream stream_options stop max_tokens max_completion_tokens modalities prediction audio presence_penalty frequency_penalty logit_bias user model custom_llm_provider response_format seed tools tool_choice max_retries logprobs top_logprobs extra_headers api_version parallel_tool_calls drop_params allowed_openai_params additional_drop_params messages reasoning_effort verbosity thinking web_search_options include_server_side_tool_invocations service_tier safety_identifier prompt_cache_key prompt_cache_retention store metadata context_management";
// LiteLLM reads these call kwargs itself, so they never reach the provider body
const REMOVED_KEYS: [&str; 5] = [
    "model",
    "stream",
    "mock_response",
    "mock_tool_calls",
    "stream_chunk_size",
];
// the executor's embedding path caps a provider answer at 32 MiB
const EMBEDDING_BODY_LIMIT: usize = 0x0200_0000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAi;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiRequest;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiResult;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiPrepared;

impl OpenAi {
    pub fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        if let OperationRequest::Chat(chat) = request {
            return chat_request(chat, deployment, catalog, false);
        }
        if let OperationRequest::Stream(chat) = request {
            return chat_request(chat, deployment, catalog, true);
        }
        if let OperationRequest::Embedding(embedding) = request {
            return embedding_request(embedding, deployment);
        }
        Err(unsupported())
    }

    pub fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        match upstream {
            Upstream::Streaming(response) => {
                if let PreparedRequest::Stream(_) = prepared {
                    return Ok(OperationResult::Stream(Box::new(response)));
                }
                Err(unsupported())
            }
            Upstream::Buffered(response) => {
                if let PreparedRequest::Chat(chat) = prepared {
                    let checked = success(response)?;
                    return decode_chat(&checked.body, chat.client_model.as_deref());
                }
                if let PreparedRequest::Embedding(embedding) = prepared {
                    let checked = success(response)?;
                    return decode_embedding(&checked.body, embedding.client_model.as_deref());
                }
                Err(unsupported())
            }
        }
    }
}

impl TransformOnUnprocessable for OpenAi {}

fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from("openai"),
        operation: String::from("provider endpoint"),
    }
}

fn chat_request(
    chat: &ChatOperation,
    deployment: &Deployment,
    catalog: &CatalogCapabilities,
    stream: bool,
) -> Result<PreparedRequest, OperationError> {
    let body = chat_body(&chat.request, &chat.model, catalog)?;
    let http = PreparedHttp {
        method: Method::POST,
        url: deployment.endpoint.clone(),
        headers: chat_headers(deployment, stream)?,
        body,
        deadline: deployment.timeout,
        body_limit: None,
    };
    let prepared = PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        client_model: chat.request.model.clone(),
    };
    Ok(if stream {
        PreparedRequest::Stream(Box::new(prepared))
    } else {
        PreparedRequest::Chat(Box::new(prepared))
    })
}

pub(crate) fn chat_body(
    request: &ChatRequest,
    model: &str,
    catalog: &CatalogCapabilities,
) -> Result<Vec<u8>, OperationError> {
    let mut requested = request_fields(request, "chat")?;
    let messages = requested
        .shift_remove("messages")
        .unwrap_or(JsonValue::Array(Vec::new()));
    for key in REMOVED_KEYS {
        requested.shift_remove(key);
    }
    let (known, forwarded): (IndexMap<String, JsonValue>, IndexMap<String, JsonValue>) = requested
        .into_iter()
        .partition(|(name, _value)| KNOWN_PARAMS.split(' ').any(|known| known == name));
    let policy = OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: OpenAiHeaders::new(),
        settings: OpenAiSettings::default(),
    };
    let supported = policy.supported_params(model, catalog);
    let mut optional: IndexMap<String, JsonValue> = IndexMap::new();
    for (name, value) in known {
        if supported.contains(&name.as_str()) {
            optional.insert(name, value);
        }
    }
    append_stream_options(request, &mut optional);
    optional.extend(forwarded);
    let mut body: IndexMap<String, JsonValue> = IndexMap::new();
    body.insert(String::from("model"), JsonValue::Text(model.to_owned()));
    body.insert(String::from("messages"), messages);
    body.extend(optional);
    serde_json::to_vec(&JsonValue::Object(body)).map_err(|source| OperationError::Encode { source })
}

fn append_stream_options(request: &ChatRequest, optional: &mut IndexMap<String, JsonValue>) {
    if !request.stream {
        return;
    }
    optional.insert(String::from("stream"), JsonValue::Bool(true));
    if optional.contains_key("stream_options") {
        return;
    }
    let mut stream_options = IndexMap::new();
    stream_options.insert(String::from("include_usage"), JsonValue::Bool(true));
    optional.insert(
        String::from("stream_options"),
        JsonValue::Object(stream_options),
    );
}

fn request_fields<T>(
    request: &T,
    label: &'static str,
) -> Result<IndexMap<String, JsonValue>, OperationError>
where
    T: Serialize,
{
    let text =
        serde_json::to_string(request).map_err(|source| OperationError::Encode { source })?;
    let value: JsonValue =
        serde_json::from_str(&text).map_err(|source| OperationError::Encode { source })?;
    let JsonValue::Object(fields) = value else {
        return Err(OperationError::Encode {
            source: JsonError::io(IoError::other(format!(
                "a {label} request did not serialize as an object"
            ))),
        });
    };
    Ok(fields)
}

fn chat_headers(deployment: &Deployment, stream: bool) -> Result<HeaderMap, OperationError> {
    let authorization = deployment
        .credential()
        .ok_or(OperationError::MissingApiKey)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        header_value(authorization.expose_secret(), "authorization")?,
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static(if stream {
            "text/event-stream"
        } else {
            "application/json"
        }),
    );
    if let Some(organization) = &deployment.organization {
        headers.insert(
            HeaderName::from_static("openai-organization"),
            header_value(organization, "openai-organization")?,
        );
    }
    Ok(headers)
}

fn embedding_request(
    embedding: &EmbeddingOperation,
    deployment: &Deployment,
) -> Result<PreparedRequest, OperationError> {
    let mut object = request_fields(&embedding.request, "embeddings")?;
    object.insert(
        String::from("model"),
        JsonValue::Text(embedding.model.clone()),
    );
    object.shift_remove("mock_response");
    object.shift_remove("mock_tool_calls");
    let bytes = serde_json::to_vec(&JsonValue::Object(object))
        .map_err(|source| OperationError::Encode { source })?;
    let authorization = deployment
        .credential()
        .ok_or(OperationError::MissingApiKey)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        header_value(authorization.expose_secret(), "authorization")?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    let http = PreparedHttp {
        method: Method::POST,
        url: deployment.endpoint_for("embeddings"),
        headers,
        body: bytes,
        deadline: deployment.timeout,
        body_limit: Some(EMBEDDING_BODY_LIMIT),
    };
    Ok(PreparedRequest::Embedding(Box::new(PreparedChat {
        http,
        retry: TransformRetry::DISABLED,
        client_model: embedding.request.model.clone(),
    })))
}

fn decode_chat(body: &[u8], client_model: Option<&str>) -> Result<OperationResult, OperationError> {
    let decoded: ChatResponse =
        serde_json::from_slice(body).map_err(|source| OperationError::Decode {
            operation: "chat",
            source,
        })?;
    let completion = decoded.for_client(client_model);
    Ok(OperationResult::Chat(ChatCompletion::OpenAi(Box::new(
        completion,
    ))))
}

fn decode_embedding(
    body: &[u8],
    client_model: Option<&str>,
) -> Result<OperationResult, OperationError> {
    let mut decoded: EmbeddingResponse =
        serde_json::from_slice(body).map_err(|source| OperationError::Decode {
            operation: "embedding",
            source,
        })?;
    if let Some(model) = client_model {
        model.clone_into(&mut decoded.model);
    }
    Ok(OperationResult::Embedding(Box::new(decoded)))
}

fn header_value(text: &str, name: &str) -> Result<HeaderValue, OperationError> {
    HeaderValue::try_from(text).map_err(|source| OperationError::InvalidHeader {
        name: name.to_owned(),
        source,
    })
}
