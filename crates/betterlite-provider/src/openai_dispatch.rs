use betterlite_http::proxy::ProxiedClient;
use betterlite_openai::chat::response::ChatResponse;
use betterlite_openai::embedding::EmbeddingResponse;
use betterlite_openai::openai_base_config::CatalogCapabilities;
use betterlite_openai::text_completion_transport::stream::CompletionStream;
use http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE};
use http::{HeaderMap, HeaderName, HeaderValue, Method};
use secrecy::ExposeSecret as _;

use crate::operation::{
    ChatCompletion, ChatOperation, EmbeddingOperation, OperationError, OperationRequest,
    OperationResult, RequestKind, success,
};
use crate::prepared::body::{
    authorization_value, chat_fields, encode, header_value, parse, request_fields,
};
use crate::prepared::deployment::Deployment;
use crate::prepared::{PreparedChat, PreparedHttp, PreparedKind, PreparedRequest, Upstream};
use crate::provider::Adapter;
use crate::transform_retry::{TransformRetry, send_with_retry};
use betterlite_models::controls::json_value::JsonValue;

pub mod call;
pub mod compose;
pub mod dispatch;
pub mod error;
#[cfg(any(test, feature = "testing"))]
pub mod fixtures;
pub mod headers;
pub mod logging;
pub mod metadata;
mod tests;
pub mod text;
pub mod transport;

// the executor's embedding path caps a provider answer at 32 MiB
const EMBEDDING_BODY_LIMIT: usize = 0x0200_0000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAi;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiRequest;

// The text branch's results; a projected text reply is a chat completion and travels in that variant instead.
pub enum OpenAiResult {
    Text(Box<JsonValue>),
    TextStream(Box<CompletionStream>),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OpenAiPrepared;

impl Adapter for OpenAi {
    const NAME: &'static str = "openai";

    fn prepare(
        &self,
        request: &OperationRequest,
        deployment: &Deployment,
        catalog: &CatalogCapabilities,
    ) -> Result<PreparedRequest, OperationError> {
        match request.kind() {
            RequestKind::Chat(chat) => chat_request(chat, deployment, catalog, false),
            RequestKind::Stream(chat) => chat_request(chat, deployment, catalog, true),
            RequestKind::Embedding(embedding) => embedding_request(embedding, deployment),
            RequestKind::Other => Err(unsupported()),
        }
    }

    async fn execute(
        &self,
        client: &ProxiedClient,
        prepared: &PreparedRequest,
        deployment: &Deployment,
    ) -> Result<Upstream, OperationError> {
        send_with_retry(client, prepared, deployment, self).await
    }

    fn decode(
        &self,
        prepared: &PreparedRequest,
        upstream: Upstream,
    ) -> Result<OperationResult, OperationError> {
        match (prepared.kind(), upstream) {
            (PreparedKind::Chat(chat), Upstream::Buffered(response)) => {
                if let Some(limit) = recovery::output_limit(chat, &response)? {
                    return Ok(limit);
                }
                let decoded: ChatResponse = parse(response, "chat")?;
                let completion = decoded.for_client(chat.client_model.as_deref());
                Ok(OperationResult::Chat(ChatCompletion::OpenAi(Box::new(
                    completion,
                ))))
            }
            (PreparedKind::Stream(_), Upstream::Streaming(response)) => {
                Ok(OperationResult::Stream(Box::new(response)))
            }
            // a refused stream arrives as a finished body, which the caller renders as the provider's own error
            (PreparedKind::Stream(chat), Upstream::Buffered(response)) => {
                if let Some(limit) = recovery::output_limit(chat, &response)? {
                    return Ok(limit);
                }
                success(response)?;
                Err(unsupported())
            }
            (PreparedKind::Embedding(embedding), Upstream::Buffered(response)) => {
                let mut decoded: EmbeddingResponse = parse(response, "embedding")?;
                decoded.model = embedding.client_model.clone().unwrap_or(decoded.model);
                Ok(OperationResult::Embedding(Box::new(decoded)))
            }
            (PreparedKind::Chat(_) | PreparedKind::Embedding(_), Upstream::Streaming(_))
            | (
                PreparedKind::Other,
                Upstream::Buffered(_) | Upstream::Streaming(_) | Upstream::Anthropic(_),
            )
            | (
                PreparedKind::Chat(_) | PreparedKind::Stream(_) | PreparedKind::Embedding(_),
                Upstream::Anthropic(_),
            ) => Err(unsupported()),
        }
    }
}

fn unsupported() -> OperationError {
    OperationError::UnsupportedOperation {
        provider: String::from(OpenAi::NAME),
        operation: String::from("provider endpoint"),
    }
}

fn chat_request(
    chat: &ChatOperation,
    deployment: &Deployment,
    catalog: &CatalogCapabilities,
    stream: bool,
) -> Result<PreparedRequest, OperationError> {
    let body = encode(chat_fields(&chat.request, &chat.model, catalog)?)?;
    let http = PreparedHttp {
        method: Method::POST,
        url: deployment.endpoint.clone(),
        headers: chat_headers(deployment, stream)?,
        body,
        deadline: deployment.timeout,
        body_limit: None,
    };
    // LiteLLM's for _ in range(2) resend loop runs only when litellm.drop_params is on
    let retry = if deployment.global_drop_params {
        TransformRetry::bounded(2)?
    } else {
        TransformRetry::DISABLED
    };
    let prepared = PreparedChat {
        http,
        retry,
        client_model: chat.request.model.clone(),
    };
    Ok(if stream {
        PreparedRequest::Stream(Box::new(prepared))
    } else {
        PreparedRequest::Chat(Box::new(prepared))
    })
}

fn chat_headers(deployment: &Deployment, stream: bool) -> Result<HeaderMap, OperationError> {
    let authorization = deployment
        .credential()
        .ok_or(OperationError::MissingApiKey)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        authorization_value(authorization.expose_secret())?,
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
    let bytes = encode(object)?;
    let authorization = deployment
        .credential()
        .ok_or(OperationError::MissingApiKey)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        authorization_value(authorization.expose_secret())?,
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

mod recovery;
