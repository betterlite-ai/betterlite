use betterlite_streaming::stream_chunk_shaping::records::GenericChunk;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Value;
use thiserror::Error;
use url::Url;

use crate::chat::response::ChatResponse;
use crate::embedding::EmbeddingResponse;
use crate::error::ProviderError;
use crate::gigachat_chat::config::GigachatBearer;
use crate::gigachat_chat::error::GigaChatError;
use crate::gigachat_chat::reply::{ReplyInput, transform_response as chat_reply};
use crate::gigachat_embedding::transform_response as embedding_reply;
use crate::gigachat_stream::{StreamEvent, chunk_parser};
use crate::gigachat_utils::{UuidIds, get_api_base};
use crate::openai_base_config::{Headers, Object};
use crate::passthrough::transformation::BasePassthroughConfig;
use crate::passthrough::transformation::types::{
    PassthroughError, PassthroughLogging, PassthroughTarget, PassthroughUrlRequest, RelayedResponse,
};

mod tests;

const ACCEPT: &str = "Accept";
const AUTHORIZATION: &str = "Authorization";
const CONTENT_TYPE: &str = "Content-Type";
const DATA_PREFIX: &str = "data: ";
const DONE: &str = "[DONE]";
const JSON_CONTENT: &str = "application/json";
const STREAM: &str = "stream";
// the pass-through logging parse has no clock, and only the body's own created value is a fact
const NO_CREATED: u64 = 0;

#[derive(Debug, Error)]
pub enum GigachatPassthroughError {
    #[error(transparent)]
    Passthrough(#[from] PassthroughError),
    #[error("{}", .0.message)]
    Provider(Box<ProviderError>),
    #[error("the GigaChat token could not be fetched: {0}")]
    Token(#[source] Box<GigaChatError>),
}

impl From<ProviderError> for GigachatPassthroughError {
    fn from(error: ProviderError) -> Self {
        Self::Provider(Box::new(error))
    }
}

#[derive(Debug)]
pub enum GigachatPassthroughLogged {
    Chat(Box<ChatResponse>),
    Embedding(Box<EmbeddingResponse>),
    Chunks(Vec<GenericChunk>),
}

// the caller resolves GIGACHAT_API_BASE and GIGACHAT_API_KEY, and an empty value counts as unset
#[derive(Debug, Clone, Default)]
pub struct GigachatPassthroughConfig {
    pub api_base_env: Option<String>,
    pub api_key_env: Option<SecretString>,
}

impl GigachatPassthroughConfig {
    #[must_use]
    pub fn get_api_key<'key>(
        &'key self,
        api_key: Option<&'key SecretString>,
    ) -> Option<&'key SecretString> {
        api_key
            .filter(|key| !key.expose_secret().is_empty())
            .or_else(|| {
                self.api_key_env
                    .as_ref()
                    .filter(|key| !key.expose_secret().is_empty())
            })
    }
}

pub async fn validate_environment(
    headers: Headers,
    credentials: Option<&SecretString>,
    bearer: &dyn GigachatBearer,
) -> Result<Headers, GigachatPassthroughError> {
    let credentials = credentials.filter(|value| !value.expose_secret().is_empty());
    let token = bearer
        .access_token(credentials)
        .await
        .map_err(|error| GigachatPassthroughError::Token(Box::new(error)))?;
    let mut merged = headers;
    merged.insert(
        String::from(AUTHORIZATION),
        format!("Bearer {}", token.expose_secret()),
    );
    merged.insert(String::from(CONTENT_TYPE), String::from(JSON_CONTENT));
    merged.insert(String::from(ACCEPT), String::from(JSON_CONTENT));
    Ok(merged)
}

// LiteLLM's get_base_model returns its argument and get_models defers to the model-info owner, so neither has an override here.
impl BasePassthroughConfig for GigachatPassthroughConfig {
    type Error = GigachatPassthroughError;
    type Logged = GigachatPassthroughLogged;

    fn is_streaming_request(&self, _endpoint: &str, request_data: &Object) -> bool {
        matches!(request_data.get(STREAM), Some(Value::Bool(true)))
    }

    fn get_complete_url(
        &self,
        request: &PassthroughUrlRequest<'_>,
    ) -> Result<(Url, String), Self::Error> {
        let base = get_api_base(request.api_base, self.api_base_env.as_deref());
        let complete = format!("{base}/{}", request.target.endpoint.trim_start_matches('/'));
        let url = Url::parse(&complete).map_err(|source| PassthroughError::InvalidUrl {
            url: complete.clone(),
            source,
        })?;
        Ok((url, base))
    }

    fn logging_non_streaming_response(
        &self,
        target: &PassthroughTarget<'_>,
        response: &RelayedResponse<'_>,
        _request_data: &Object,
        _logging_obj: &mut dyn PassthroughLogging,
    ) -> Option<Self::Logged> {
        if target.endpoint.contains("completions") {
            let input = ReplyInput {
                status_code: response.status_code,
                body: response.body,
                model: target.model,
                created: NO_CREATED,
                structured_output: false,
            };
            return chat_reply(&UuidIds, &input)
                .ok()
                .map(|reply| GigachatPassthroughLogged::Chat(Box::new(reply)));
        }
        if target.endpoint.contains("embeddings") {
            return embedding_reply(response.body)
                .ok()
                .map(|reply| GigachatPassthroughLogged::Embedding(Box::new(reply)));
        }
        None
    }

    fn handle_collected_chunks(
        &self,
        all_chunks: &[String],
        _logging_obj: &mut dyn PassthroughLogging,
        _target: &PassthroughTarget<'_>,
    ) -> Option<Self::Logged> {
        let mut chunks = Vec::new();
        for chunk in all_chunks {
            let line = chunk.trim();
            if line == DONE {
                continue;
            }
            let payload = line.strip_prefix(DATA_PREFIX).unwrap_or(line);
            let Ok(Value::Object(_)) = serde_json::from_str::<Value>(payload) else {
                continue;
            };
            match chunk_parser(payload.as_bytes(), &UuidIds) {
                Ok(StreamEvent::Chunk(parsed)) => chunks.push(*parsed),
                Ok(StreamEvent::Done) | Err(_) => continue,
            }
        }
        (!chunks.is_empty()).then_some(GigachatPassthroughLogged::Chunks(chunks))
    }
}
