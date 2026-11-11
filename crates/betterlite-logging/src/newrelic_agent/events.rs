use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use serde_json::Number;

use crate::newrelic_extraction::request_params::RequestParameters;
use crate::newrelic_extraction::response_fields::Usage;

pub const SUMMARY_EVENT_NAME: &str = "LlmChatCompletionSummary";
pub const MESSAGE_EVENT_NAME: &str = "LlmChatCompletionMessage";
pub const ERROR_METRIC_NAME: &str = "LLM/LiteLLM/Error";
pub const SUPPORTABILITY_METRIC_PREFIX: &str = "Supportability/Python/ML/LiteLLM/";
pub const INGEST_SOURCE: &str = "litellm";

pub struct SummaryEvent {
    pub request_id: String,
    pub trace_id: String,
    pub request_model: String,
    pub response_model: String,
    pub finish_reason: String,
    pub number_of_messages: usize,
    pub vendor: String,
    pub usage: Usage,
    pub duration: Option<f64>,
    pub request_parameters: RequestParameters,
}

impl SummaryEvent {
    #[must_use]
    pub fn attributes(&self) -> IndexMap<String, Json> {
        let mut attributes = self.identity_attributes();
        attributes.extend(
            self.usage_attributes()
                .map(|(name, value)| (name.to_owned(), value)),
        );
        self.optional_attributes(&mut attributes);
        attributes
    }

    fn identity_attributes(&self) -> IndexMap<String, Json> {
        let mut attributes = IndexMap::new();
        attributes.insert("id".to_owned(), Json::Text(self.request_id.clone()));
        attributes.insert("request_id".to_owned(), Json::Text(self.request_id.clone()));
        attributes.insert(
            "request.model".to_owned(),
            Json::Text(self.request_model.clone()),
        );
        attributes.insert(
            "response.model".to_owned(),
            Json::Text(self.response_model.clone()),
        );
        attributes.insert(
            "response.choices.finish_reason".to_owned(),
            Json::Text(self.finish_reason.clone()),
        );
        attributes.insert(
            "response.number_of_messages".to_owned(),
            Json::Number(Number::from(self.number_of_messages)),
        );
        attributes.insert("vendor".to_owned(), Json::Text(self.vendor.clone()));
        attributes.insert(
            "ingest_source".to_owned(),
            Json::Text(INGEST_SOURCE.to_owned()),
        );
        attributes
    }

    fn usage_attributes(&self) -> impl Iterator<Item = (&'static str, Json)> {
        [
            (
                "response.usage.prompt_tokens",
                Json::Number(Number::from(self.usage.prompt_tokens)),
            ),
            (
                "response.usage.completion_tokens",
                Json::Number(Number::from(self.usage.completion_tokens)),
            ),
            (
                "response.usage.total_tokens",
                Json::Number(Number::from(self.usage.total_tokens)),
            ),
        ]
        .into_iter()
    }

    fn optional_attributes(&self, attributes: &mut IndexMap<String, Json>) {
        // LiteLLM always has a trace id here, so the attribute is only absent for an empty one
        if !self.trace_id.is_empty() {
            attributes.insert("trace_id".to_owned(), Json::Text(self.trace_id.clone()));
        }
        // A JSON number cannot carry a non-finite duration, so one is left out of the event
        if let Some(duration) = self.duration.and_then(Number::from_f64) {
            attributes.insert("duration".to_owned(), Json::Number(duration));
        }
        if let Some(temperature) = &self.request_parameters.temperature {
            attributes.insert("request.temperature".to_owned(), temperature.clone());
        }
        if let Some(max_tokens) = &self.request_parameters.max_tokens {
            attributes.insert("request.max_tokens".to_owned(), max_tokens.clone());
        }
    }
}

pub struct ChatMessageEvent {
    pub id: String,
    pub request_id: String,
    pub trace_id: String,
    pub role: String,
    pub sequence: usize,
    pub response_model: String,
    pub vendor: String,
    pub content: Option<String>,
    pub is_response: bool,
    pub timestamp: Option<i64>,
}

impl ChatMessageEvent {
    #[must_use]
    pub fn attributes(&self) -> IndexMap<String, Json> {
        let mut attributes = IndexMap::new();
        attributes.insert("id".to_owned(), Json::Text(self.id.clone()));
        attributes.insert("request_id".to_owned(), Json::Text(self.request_id.clone()));
        attributes.insert(
            "completion_id".to_owned(),
            Json::Text(self.request_id.clone()),
        );
        if !self.trace_id.is_empty() {
            attributes.insert("trace_id".to_owned(), Json::Text(self.trace_id.clone()));
        }
        attributes.insert("role".to_owned(), Json::Text(self.role.clone()));
        attributes.insert(
            "sequence".to_owned(),
            Json::Number(Number::from(self.sequence)),
        );
        attributes.insert(
            "response.model".to_owned(),
            Json::Text(self.response_model.clone()),
        );
        attributes.insert("vendor".to_owned(), Json::Text(self.vendor.clone()));
        attributes.insert(
            "ingest_source".to_owned(),
            Json::Text(INGEST_SOURCE.to_owned()),
        );
        // LiteLLM never reports per-message token counts, so every message says zero
        attributes.insert("token_count".to_owned(), Json::Number(Number::from(0_i64)));
        if let Some(content) = &self.content {
            attributes.insert("content".to_owned(), Json::Text(content.clone()));
        }
        // The schema omits the flag for request messages and only marks response messages
        if self.is_response {
            attributes.insert("is_response".to_owned(), Json::Bool(true));
        }
        if let Some(timestamp) = self.timestamp {
            attributes.insert(
                "timestamp".to_owned(),
                Json::Number(Number::from(timestamp)),
            );
        }
        attributes
    }
}
