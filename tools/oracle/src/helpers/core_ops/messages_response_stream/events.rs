use betterlite_core::api::messages_response_stream::source::{
    ProviderError, ProviderEvent, ProviderResponse, ResponsesEvent,
};
use betterlite_models::decisions::Json;
use betterlite_openai::api::responses_objects::ResponseApiUsage;
use indexmap::IndexMap;
use serde::Deserialize;

// The two event shapes __anext__ can pull: a decoded mapping, or a provider object whose attributes getattr reads.
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum EventShape {
    Mapping,
    Attributes,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EventSpec {
    shape: EventShape,
    #[serde(default)]
    fields: Option<IndexMap<String, Json>>,
    #[serde(default)]
    attributes: Option<AttributesSpec>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttributesSpec {
    #[serde(rename = "type", default)]
    event_type: Option<Json>,
    #[serde(default)]
    item: Option<Json>,
    #[serde(default)]
    item_id: Option<Json>,
    #[serde(default)]
    delta: Option<Json>,
    #[serde(default)]
    summary_index: Option<Json>,
    #[serde(default)]
    response: Option<ResponseSpec>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseSpec {
    #[serde(default)]
    output: Vec<Json>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    usage: Option<ResponseApiUsage>,
    #[serde(default)]
    error: Option<ErrorSpec>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ErrorSpec {
    #[serde(default)]
    message: Option<Json>,
    #[serde(rename = "type", default)]
    error_type: Option<Json>,
    #[serde(default)]
    code: Option<Json>,
}

impl EventSpec {
    pub(super) fn event(&self) -> ResponsesEvent {
        match self.shape {
            EventShape::Mapping => ResponsesEvent {
                attributes: ProviderEvent::default(),
                mapping: self.fields.clone(),
            },
            EventShape::Attributes => ResponsesEvent {
                attributes: self
                    .attributes
                    .as_ref()
                    .map_or_else(ProviderEvent::default, AttributesSpec::event),
                mapping: None,
            },
        }
    }
}

impl AttributesSpec {
    fn event(&self) -> ProviderEvent {
        ProviderEvent {
            event_type: self.event_type.clone(),
            item: self.item.clone(),
            item_id: self.item_id.clone(),
            delta: self.delta.clone(),
            summary_index: self.summary_index.clone(),
            response: self.response.as_ref().map(ResponseSpec::response),
        }
    }
}

impl ResponseSpec {
    fn response(&self) -> ProviderResponse {
        ProviderResponse {
            output: self.output.clone(),
            status: self.status.clone(),
            usage: self.usage.clone(),
            error: self.error.as_ref().map(ErrorSpec::error),
        }
    }
}

impl ErrorSpec {
    fn error(&self) -> ProviderError {
        ProviderError {
            message: self.message.clone(),
            error_type: self.error_type.clone(),
            code: self.code.clone(),
        }
    }
}
