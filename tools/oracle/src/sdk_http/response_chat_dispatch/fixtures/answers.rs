use async_trait::async_trait;
use betterlite_core::api::messages_stream_split::errors::StreamFailure;
use betterlite_core::api::response_chat_dispatch::port::{
    Answer, BridgeResponse, ResponsePayload, ResponsesEventStream,
};
use betterlite_core::api::responses_create::bridge::Created;
use betterlite_core::api::responses_stream_driver::source::{
    PayloadSource, PrefetchedSource, SourceFailure,
};
use betterlite_models::decisions::HiddenParams;
use betterlite_openai::api::responses_objects::ResponsesApiResponse;
use serde_json::{Map, Value};

use super::execution::ExecutionFailure;
use crate::sdk_http::error::SdkHttpError;

// The bridge's four answers over B1078's create result, with the hidden maps the bridge merges
pub(crate) fn answer(created: Created<()>) -> Result<Answer, ExecutionFailure> {
    match created {
        Created::Responses(stamped) => Ok(Answer::Response(Box::new(BridgeResponse {
            payload: ResponsePayload::Typed(Box::new(stamped.response)),
            existing_hidden: hidden_map(&stamped.hidden)?,
            stream_hidden: Map::new(),
        }))),
        Created::Raw(raw) => Ok(Answer::Response(Box::new(BridgeResponse {
            payload: ResponsePayload::Dict(
                serde_json::from_str(raw.get()).map_err(|error| invalid(&error.to_string()))?,
            ),
            existing_hidden: Map::new(),
            stream_hidden: Map::new(),
        }))),
        Created::Live(live) => Ok(Answer::Events(Box::new(LiveEvents {
            source: live.source,
            hidden: Map::new(),
        }))),
        Created::Synthetic(synthetic) => synthetic_answer(&synthetic.response),
        Created::Chat(_outcome) => Err(ExecutionFailure::Sdk(invalid(
            "a chat outcome is not a responses answer",
        ))),
    }
}

// A provider without native streaming replays its completed response as the one event the iterator reads
fn synthetic_answer(response: &ResponsesApiResponse) -> Result<Answer, ExecutionFailure> {
    let payload = serde_json::to_value(response).map_err(|error| invalid(&error.to_string()))?;
    let mut event = Map::new();
    event.insert(
        String::from("type"),
        Value::String(String::from("response.completed")),
    );
    event.insert(String::from("response"), payload);
    let bytes =
        serde_json::to_vec(&Value::Object(event)).map_err(|error| invalid(&error.to_string()))?;
    Ok(Answer::Events(Box::new(LiveEvents {
        source: Box::new(PrefetchedSource::new(vec![Ok(bytes)])),
        hidden: Map::new(),
    })))
}

fn hidden_map(hidden: &HiddenParams) -> Result<Map<String, Value>, ExecutionFailure> {
    let value = serde_json::to_value(hidden).map_err(|error| invalid(&error.to_string()))?;
    match value {
        Value::Object(map) => Ok(map),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Array(_) => {
            Err(ExecutionFailure::Sdk(invalid(
                "the response hidden parameters are an object",
            )))
        }
    }
}

struct LiveEvents {
    source: Box<dyn PayloadSource>,
    hidden: Map<String, Value>,
}

#[async_trait]
impl ResponsesEventStream for LiveEvents {
    async fn next_event(&mut self) -> Option<Result<Vec<u8>, StreamFailure>> {
        match self.source.next_payload().await {
            Some(Ok(payload)) => Some(Ok(payload)),
            Some(Err(failure)) => Some(Err(StreamFailure::Unclassified {
                text: source_text(failure),
            })),
            None => None,
        }
    }

    fn hidden_params(&self) -> &Map<String, Value> {
        &self.hidden
    }
}

fn source_text(failure: SourceFailure) -> String {
    match failure {
        SourceFailure::Transport(error) => error.message,
        SourceFailure::Decode(text) => text,
    }
}

fn invalid(message: &str) -> SdkHttpError {
    SdkHttpError::Invalid(message.to_owned())
}
