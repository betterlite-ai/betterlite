use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::logging_success::types::SuccessResult;

// LiteLLM's StandardPassThroughResponseObject: response is a str | dict
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PassThroughResponseObject {
    pub response: Json,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpstreamBodyText {
    Read(String),
    NotRead,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PassThroughResponse {
    ModelResponse(Json),
    TextCompletion(Json),
    Image(Json),
    Embedding(Json),
    VideoOperation(Json),
    Standard(PassThroughResponseObject),
    ResponsesApi(Json),
    Transcription(Json),
    VertexBatchRecord(Json),
    VertexSearchWrapper(PassThroughResponseObject),
}

impl PassThroughResponse {
    #[must_use]
    pub fn into_json(self) -> Json {
        match self {
            Self::ModelResponse(value)
            | Self::TextCompletion(value)
            | Self::Image(value)
            | Self::Embedding(value)
            | Self::VideoOperation(value)
            | Self::ResponsesApi(value)
            | Self::Transcription(value)
            | Self::VertexBatchRecord(value) => value,
            Self::Standard(value) | Self::VertexSearchWrapper(value) => {
                let mut object = IndexMap::new();
                object.insert("response".to_owned(), value.response);
                Json::Object(object)
            }
        }
    }
}

impl From<PassThroughResponse> for SuccessResult {
    fn from(response: PassThroughResponse) -> Self {
        Self::Other(response.into_json())
    }
}

#[must_use]
pub fn safe_response_text(body: &UpstreamBodyText) -> String {
    match body {
        UpstreamBodyText::Read(text) => text.clone(),
        UpstreamBodyText::NotRead => String::new(),
    }
}
