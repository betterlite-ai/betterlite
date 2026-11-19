use std::time::Duration;

use betterlite_http::proxy::ProxiedClient;
use betterlite_http::proxy::outbound::{OutboundRequest, RequestBody};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::credentials::is_openai_backed_api_base;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers as OpenAiHeaders, Object, OpenAiConfig,
    OpenAiSettings,
};
use betterlite_openai::proxy::{map_params, transform_request};
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderName, HeaderValue};
use http::{HeaderMap, Method};
use secrecy::ExposeSecret as _;
use url::Url;

use crate::operation::{
    ChatCompletion, ChatOperation, EmbeddingOperation, OperationError, OperationRequest,
    OperationResult, RequestKind, success,
};
use crate::prepared::Upstream;
use crate::prepared::body::{
    self, authorization_value, encode, header_value, parse, request_fields,
};
use crate::prepared::deployment::Deployment;

use super::Legacy;
use super::error;

#[derive(Debug, Clone)]
pub(super) struct Call {
    pub(super) url: Url,
    pub(super) headers: HeaderMap,
    pub(super) body: Vec<u8>,
    pub(super) timeout: Duration,
    kind: Kind,
    client_model: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum Kind {
    Chat,
    Stream,
    Embedding,
}

pub(super) fn prepare(
    request: &OperationRequest,
    deployment: &Deployment,
    catalog: &CatalogCapabilities,
) -> Result<Call, OperationError> {
    match request.kind() {
        RequestKind::Chat(chat) => chat_call(chat, deployment, catalog, false),
        RequestKind::Stream(chat) => chat_call(chat, deployment, catalog, true),
        RequestKind::Embedding(embedding) => embedding_call(embedding, deployment),
        RequestKind::Other => Err(OperationError::UnsupportedOperation {
            provider: String::from(Legacy::LitellmProxy.slug()),
            operation: String::from("provider endpoint"),
        }),
    }
}

// LiteLLMProxyChatConfig sends the model and messages untouched and every supported call param beside them
fn chat_call(
    chat: &ChatOperation,
    deployment: &Deployment,
    catalog: &CatalogCapabilities,
    stream: bool,
) -> Result<Call, OperationError> {
    let body::ChatRequestFields { messages, params } = body::chat_request_fields(&chat.request)?;
    let serialized = serde_json::to_value(JsonValue::Object(params))
        .map_err(|source| OperationError::Encode { source })?;
    let requested: Object =
        serde_json::from_value(serialized).map_err(|source| OperationError::Encode { source })?;
    let config = OpenAiConfig {
        family: ConfigFamily::OpenAi,
        api_key: None,
        headers: OpenAiHeaders::new(),
        settings: OpenAiSettings::default(),
    };
    let mut optional = Object::new();
    map_params(
        &config,
        &deployment.model,
        &requested,
        &mut optional,
        catalog,
    )
    .map_err(error::proxy)?;
    if stream {
        optional.insert(String::from("stream"), serde_json::Value::Bool(true));
        // get_stream_options asks for usage only from an OpenAI-backed base, and the caller's own map wins
        if !optional.contains_key("stream_options")
            && is_openai_backed_api_base(&deployment.api_base).unwrap_or(false)
        {
            let mut usage = Object::new();
            usage.insert(String::from("include_usage"), serde_json::Value::Bool(true));
            optional.insert(
                String::from("stream_options"),
                serde_json::Value::Object(usage),
            );
        }
    }
    let messages =
        serde_json::to_value(messages).map_err(|source| OperationError::Encode { source })?;
    let body = transform_request(&deployment.model, messages, &optional);
    let mut headers = HeaderMap::new();
    let credential = deployment
        .credential()
        .ok_or(OperationError::MissingApiKey)?;
    headers.insert(
        AUTHORIZATION,
        authorization_value(credential.expose_secret())?,
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
    Ok(Call {
        url: deployment.endpoint.clone(),
        headers,
        body: serde_json::to_vec(&body).map_err(|source| OperationError::Encode { source })?,
        timeout: deployment.timeout,
        kind: if stream { Kind::Stream } else { Kind::Chat },
        client_model: chat.request.model.clone(),
    })
}

// the proxy embedding branch runs the OpenAI embedding handler after B006's chain resolved the deployment
fn embedding_call(
    embedding: &EmbeddingOperation,
    deployment: &Deployment,
) -> Result<Call, OperationError> {
    let mut fields = request_fields(&embedding.request, "embeddings")?;
    fields.insert(
        String::from("model"),
        JsonValue::Text(embedding.model.clone()),
    );
    fields.shift_remove("mock_response");
    fields.shift_remove("mock_tool_calls");
    let explicit = match fields.get("encoding_format") {
        Some(JsonValue::Text(text)) => Some(text.clone()),
        Some(
            JsonValue::Null
            | JsonValue::Bool(_)
            | JsonValue::Number(_)
            | JsonValue::Array(_)
            | JsonValue::Object(_),
        )
        | None => None,
    };
    // the loaded section reaches this path with the settings seam; until then the default is unset
    let configured = betterlite_config::sections::openai_provider::Settings::default();
    let requested = explicit.or_else(|| configured.embedding_format.clone());
    match requested.filter(|format| format.trim().to_lowercase() != "none") {
        Some(format) => {
            fields.insert(String::from("encoding_format"), JsonValue::Text(format));
        }
        None => {
            fields.shift_remove("encoding_format");
        }
    }
    let mut headers = HeaderMap::new();
    let credential = deployment
        .credential()
        .ok_or(OperationError::MissingApiKey)?;
    headers.insert(
        AUTHORIZATION,
        authorization_value(credential.expose_secret())?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(organization) = &deployment.organization {
        headers.insert(
            HeaderName::from_static("openai-organization"),
            header_value(organization, "openai-organization")?,
        );
    }
    Ok(Call {
        url: deployment.endpoint_for("embeddings"),
        headers,
        body: encode(fields)?,
        timeout: deployment.timeout,
        kind: Kind::Embedding,
        // the proxy restamps the embedding answer with the name the client asked for
        client_model: embedding.request.model.clone(),
    })
}

pub(super) async fn execute(
    client: &ProxiedClient,
    call: &Call,
) -> Result<Upstream, OperationError> {
    let request = OutboundRequest {
        method: Method::POST,
        url: call.url.clone(),
        query: Vec::new(),
        headers: call.headers.clone(),
        body: RequestBody::Json(call.body.clone()),
        deadline: call.timeout,
    };
    // the OpenAI client follows redirects, so the proxy route does too
    let response = client
        .send_following(request)
        .await
        .map_err(OperationError::from)?;
    if matches!(call.kind, Kind::Stream) && response.response.status().is_success() {
        return Ok(Upstream::Streaming(response));
    }
    let buffered = response
        .buffered(client.limits.body_bytes)
        .await
        .map_err(OperationError::from)?;
    Ok(Upstream::Buffered(buffered))
}

pub(super) fn decode(call: &Call, upstream: Upstream) -> Result<OperationResult, OperationError> {
    match (call.kind, upstream) {
        (Kind::Chat, Upstream::Buffered(response)) => {
            let decoded: ChatResponse = parse(response, "chat")?;
            let completion = decoded.for_client(call.client_model.as_deref());
            Ok(OperationResult::Chat(ChatCompletion::OpenAi(Box::new(
                completion,
            ))))
        }
        (Kind::Stream, Upstream::Streaming(response)) => {
            Ok(OperationResult::Stream(Box::new(response)))
        }
        // a refused stream arrives as a finished body, which the caller renders as the provider's own error
        (Kind::Stream, Upstream::Buffered(response)) => {
            success(response)?;
            Err(OperationError::UnsupportedOperation {
                provider: String::from(Legacy::LitellmProxy.slug()),
                operation: String::from("stream"),
            })
        }
        (Kind::Embedding, Upstream::Buffered(response)) => {
            let mut decoded: EmbeddingResponse = parse(response, "embedding")?;
            if let Some(model) = call
                .client_model
                .as_deref()
                .filter(|model| !model.is_empty())
            {
                decoded.model = String::from(model);
            }
            Ok(OperationResult::Embedding(Box::new(decoded)))
        }
        (Kind::Chat | Kind::Embedding, Upstream::Streaming(_)) => {
            Err(OperationError::UnsupportedOperation {
                provider: String::from(Legacy::LitellmProxy.slug()),
                operation: String::from("provider endpoint"),
            })
        }
    }
}
