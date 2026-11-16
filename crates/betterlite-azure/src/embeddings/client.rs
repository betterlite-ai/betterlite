use std::collections::BTreeMap;
use std::future::Future;

use betterlite_models::decisions::Json;
use betterlite_openai::embedding::EmbeddingResponse;
use indexmap::IndexMap;
use serde_json::error::Category;
use thiserror::Error;

use crate::endpoints::projection::{ClientKind, ClientOutcome};

// header names are lower case, the way httpx lists them
pub type Headers = BTreeMap<String, String>;
pub type Fields = IndexMap<String, Json>;

pub struct RawEmbedding {
    pub status: u16,
    pub headers: Headers,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct ErrorResponse {
    pub headers: Headers,
    pub text: String,
}

// the attributes of a raised SDK exception that the Azure handler reads
#[derive(Debug, Clone, Error)]
#[error("{message}")]
pub struct SdkError {
    pub status_code: Option<u16>,
    pub headers: Option<Headers>,
    pub response: Option<ErrorResponse>,
    pub message: String,
}

impl SdkError {
    // the SDK raises this for a 4xx or 5xx answer, naming a JSON body after the status and any other text as is
    #[must_use]
    pub fn status(status: u16, headers: Headers, text: String) -> Self {
        let trimmed = text.trim();
        let message = if serde_json::from_str::<Json>(trimmed).is_ok() {
            format!("Error code: {status} - {trimmed}")
        } else if trimmed.is_empty() {
            format!("Error code: {status}")
        } else {
            trimmed.to_owned()
        };
        Self {
            status_code: Some(status),
            headers: None,
            message,
            response: Some(ErrorResponse { headers, text }),
        }
    }
}

pub enum Parsed {
    Embedding(Box<EmbeddingResponse>),
    Text(String),
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("{0}")]
    Json(#[source] serde_json::Error),
    #[error("the answer is not an embeddings response: {0}")]
    Shape(#[source] serde_json::Error),
}

impl RawEmbedding {
    // a body that is not JSON under a non-JSON content type comes back as text, as in the OpenAI SDK's parse
    pub fn parse(&self) -> Result<Parsed, ParseError> {
        let content_type = self.headers.get("content-type").map_or("*", String::as_str);
        let media_type = content_type
            .split_once(';')
            .map_or(content_type, |(kind, _parameters)| kind);
        match serde_json::from_slice::<EmbeddingResponse>(&self.body) {
            Ok(embedding) => Ok(Parsed::Embedding(Box::new(embedding))),
            Err(error) if matches!(error.classify(), Category::Data) => {
                Err(ParseError::Shape(error))
            }
            Err(error) if media_type.ends_with("json") => Err(ParseError::Json(error)),
            Err(_text_body) => Ok(Parsed::Text(str::from_utf8(&self.body).map_or_else(
                |_source| format!("{} bytes that are not UTF-8", self.body.len()),
                str::to_owned,
            ))),
        }
    }
}

pub trait EmbeddingClient: Sync {
    fn kind(&self) -> ClientKind;

    // embeddings.with_raw_response.create(**data, timeout=timeout); a 4xx or 5xx answer is an Err
    fn create(
        &self,
        data: &Fields,
        timeout: f64,
    ) -> impl Future<Output = Result<RawEmbedding, SdkError>> + Send;
}

pub trait ClientFactory: Sync {
    type Client: EmbeddingClient + Send;

    fn client(&self, outcome: &ClientOutcome<'_>) -> Result<Self::Client, SdkError>;
}
