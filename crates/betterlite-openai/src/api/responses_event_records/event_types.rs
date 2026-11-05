use betterlite_models::decisions::Json;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub type Extra = BTreeMap<String, Json>;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequiredNullable<T> {
    Null,
    Value(T),
}

macro_rules! event_record {
    ($name:ident { $($(#[$attribute:meta])* $field:ident: $kind:ty),* $(,)? }) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        pub struct $name {
            $($(#[$attribute])* pub $field: $kind,)*
            #[serde(flatten)]
            pub extra: Extra,
        }
    };
}

event_record!(ResponseEvent {
    response: super::super::responses_objects::ResponsesApiResponse,
});
event_record!(ResponsePartAddedEvent {
    item_id: String,
    output_index: i64,
    part: Extra,
});
event_record!(ReasoningSummaryTextDeltaEvent {
    item_id: String,
    output_index: i64,
    #[serde(default)]
    summary_index: i64,
    delta: String,
});
event_record!(ReasoningSummaryTextDoneEvent {
    item_id: String,
    output_index: i64,
    sequence_number: i64,
    #[serde(default)]
    summary_index: i64,
    text: String,
});
event_record!(ReasoningSummaryPartDoneEvent {
    item_id: String,
    output_index: i64,
    sequence_number: i64,
    #[serde(default)]
    summary_index: i64,
    part: Extra,
});
event_record!(OutputItemAddedEvent {
    output_index: i64,
    item: RequiredNullable<Extra>,
});
event_record!(OutputItemDoneEvent {
    output_index: i64,
    #[serde(default = "default_sequence_number")]
    sequence_number: i64,
    item: Extra,
});

const fn default_sequence_number() -> i64 {
    1
}

event_record!(ContentPartAddedEvent {
    item_id: String,
    output_index: i64,
    content_index: i64,
    part: Extra,
});
event_record!(ContentPartDoneEvent {
    item_id: String,
    output_index: i64,
    content_index: i64,
    part: ContentPartDonePart,
});

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentPartDonePart {
    #[serde(rename = "output_text")]
    OutputText {
        text: String,
        annotations: Vec<Extra>,
        logprobs: RequiredNullable<Vec<ResponseLogprob>>,
        #[serde(flatten)]
        extra: Extra,
    },
    #[serde(rename = "refusal")]
    Refusal {
        refusal: String,
        #[serde(flatten)]
        extra: Extra,
    },
    #[serde(rename = "reasoning_text")]
    ReasoningText {
        reasoning: String,
        #[serde(flatten)]
        extra: Extra,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseLogprob {
    pub token: String,
    pub logprob: serde_json::Number,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "serde_with::As::<serde_with::FromInto<Vec<Json>>>::deserialize"
    )]
    pub bytes: Option<Vec<Json>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "serde_with::As::<serde_with::FromInto<Vec<ResponseTopLogprob>>>::deserialize"
    )]
    pub top_logprobs: Option<Vec<ResponseTopLogprob>>,
    #[serde(flatten)]
    pub extra: Extra,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResponseTopLogprob {
    pub token: String,
    pub logprob: serde_json::Number,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "serde_with::As::<serde_with::FromInto<Vec<Json>>>::deserialize"
    )]
    pub bytes: Option<Vec<Json>>,
    #[serde(flatten)]
    pub extra: Extra,
}

event_record!(OutputTextDeltaEvent {
    item_id: String,
    output_index: i64,
    content_index: i64,
    delta: String,
});
event_record!(OutputTextAnnotationAddedEvent {
    item_id: String,
    output_index: i64,
    content_index: i64,
    annotation_index: i64,
    annotation: Extra,
});
event_record!(OutputTextDoneEvent {
    item_id: String,
    output_index: i64,
    content_index: i64,
    text: String,
});
event_record!(RefusalDeltaEvent {
    item_id: String,
    output_index: i64,
    content_index: i64,
    delta: String,
});
event_record!(RefusalDoneEvent {
    item_id: String,
    output_index: i64,
    content_index: i64,
    refusal: String,
});
event_record!(FunctionArgsDelta {
    item_id: String,
    output_index: i64,
    delta: String,
});
event_record!(FunctionArgsDone {
    item_id: String,
    output_index: i64,
    arguments: String,
});
event_record!(IndexedItemEvent {
    output_index: i64,
    item_id: String,
});
event_record!(McpListToolsEvent {
    sequence_number: i64,
    output_index: i64,
    item_id: String,
});
event_record!(McpCallArgumentsDeltaEvent {
    output_index: i64,
    item_id: String,
    delta: String,
    sequence_number: i64,
});
event_record!(McpCallArgumentsDoneEvent {
    output_index: i64,
    item_id: String,
    arguments: String,
    sequence_number: i64,
});
event_record!(McpCallTerminalEvent {
    sequence_number: i64,
    item_id: String,
    output_index: i64,
});
event_record!(ImagePartial {
    partial_image_index: i64,
    b64_json: String,
});
event_record!(ErrorEvent {
    sequence_number: i64,
    error: ErrorEventError,
});
event_record!(ErrorEventError {
    r#type: String,
    code: String,
    message: String,
    #[serde(default)] param: Option<ErrorEventParam>,
});

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ErrorEventParam {
    Text(String),
    Fields(Extra),
}
