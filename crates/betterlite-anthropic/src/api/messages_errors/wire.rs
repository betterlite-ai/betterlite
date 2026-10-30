use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::fields::{Fields, encode};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnthropicMetadata {
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    StopSequence,
    ToolUse,
    Refusal,
    Compaction,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentKind {
    Text,
    ToolUse,
    Thinking,
    RedactedThinking,
    Compaction,
}

pub trait Missing {
    fn is_missing(&self) -> bool;
}

// Concrete conversions let Serde derive every wire operation without generic bound overrides.
macro_rules! tolerant {
    ($name:ident, $value:ty) => {
        #[derive(Debug, Clone, Default, Deserialize, Serialize)]
        #[serde(untagged, from = "Box<RawValue>")]
        pub enum $name {
            #[default]
            Missing,
            Known($value),
            Other(Box<RawValue>),
        }
        impl From<Box<RawValue>> for $name {
            fn from(raw: Box<RawValue>) -> Self {
                serde_json::from_str(raw.get()).map_or_else(|_| Self::Other(raw), Self::Known)
            }
        }
        impl Missing for $name {
            fn is_missing(&self) -> bool {
                match self {
                    Self::Missing => true,
                    Self::Known(_) | Self::Other(_) => false,
                }
            }
        }
    };
}

tolerant!(TextField, String);
tolerant!(CountField, u64);
tolerant!(ObjectField, Fields);
tolerant!(ObjectsField, Vec<Fields>);
tolerant!(RawField, Box<RawValue>);
tolerant!(ContentField, Vec<ContentRecord>);
tolerant!(UsageField, Box<UsageRecord>);
tolerant!(ErrorField, Box<DetailRecord>);
tolerant!(StopField, Box<StopReason>);
tolerant!(KindField, Box<ContentKind>);

// Keep a typed view beside the original fields, whose order and unknown values belong on the wire.
macro_rules! wire_record {
    ($name:ident, $head:ident) => {
        #[derive(Debug, Clone, Serialize, Deserialize)]
        #[serde(try_from = "Fields")]
        pub struct $name {
            #[serde(skip)]
            pub head: $head,
            #[serde(flatten)]
            pub fields: Fields,
        }
        impl TryFrom<Fields> for $name {
            type Error = serde_json::Error;
            fn try_from(fields: Fields) -> Result<Self, Self::Error> {
                let json = serde_json::to_string(&fields)?;
                let head = serde_json::from_str(&json)?;
                Ok(Self { head, fields })
            }
        }
        impl $name {
            pub fn from_head(head: $head) -> Result<Self, serde_json::Error> {
                let fields = encode(&head)?;
                Ok(Self { head, fields })
            }
        }
    };
}

wire_record!(ErrorRecord, AnthropicErrorResponse);
wire_record!(DetailRecord, AnthropicErrorDetail);
wire_record!(ResponseRecord, AnthropicMessagesResponse);
wire_record!(ContentRecord, ContentBlock);
wire_record!(UsageRecord, MessageUsage);

// TypedDict does not validate runtime values, and explicit null differs from an absent key.
macro_rules! record {
    ($name:ident { $($field:ident: $value:ident),* $(,)? }) => {
        #[derive(Debug, Clone, Default, Serialize, Deserialize)]
        pub struct $name {
            $(#[serde(default, skip_serializing_if = "Missing::is_missing")]
            pub $field: $value,)*
        }
    };
}
record!(MessageUsage {
    input_tokens: CountField,
    output_tokens: CountField,
    iterations: ObjectsField,
    cache_creation_input_tokens: CountField,
    cache_read_input_tokens: CountField,
    server_tool_use: ObjectField,
});
record!(ContentBlock {
    r#type: KindField,
    text: TextField,
    citations: ObjectsField,
    id: TextField,
    input: ObjectField,
    name: TextField,
    thinking: TextField,
    signature: TextField,
    data: TextField,
    content: TextField,
});
record!(AnthropicMessagesResponse {
    r#type: TextField,
    content: ContentField,
    id: TextField,
    model: TextField,
    role: TextField,
    stop_reason: StopField,
    stop_details: ObjectField,
    stop_sequence: TextField,
    usage: UsageField,
    context_management: ObjectField,
    safeguard_results: ObjectsField,
});
record!(AnthropicErrorDetail {
    r#type: TextField,
    message: TextField,
    provider_specific_fields: ObjectField,
    litellm_call_id: TextField,
});
record!(AnthropicErrorResponse {
    r#type: TextField,
    error: ErrorField,
    request_id: TextField,
});

#[derive(Debug, Deserialize)]
pub(super) struct ProviderMessage {
    #[serde(default)]
    pub detail: ObjectField,
    #[serde(default, rename = "Message")]
    pub aws: RawField,
    #[serde(default)]
    pub message: RawField,
}
