use betterlite_guardrails::bedrock_message_selection::SelectionContext;
use betterlite_guardrails::bedrock_message_selection::wire::{Message, ModelResponse};
use betterlite_guardrails::bedrock_records::{
    MessageFilterResult, MessageSelection, QualifiedTextBlock, Qualifier, Response, Source,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub(crate) struct Input {
    pub(super) step: Step,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Step {
    Format(Format),
    Grounding(GroundingInput),
    Selection(SelectionInput),
    Prepare(Prepare),
    Merge(Merge),
    MergeMessages(MergeMessages),
    Extract(Extract),
    MaskMessages(MaskMessages),
    MaskResponse(MaskResponse),
}

#[derive(Debug, Deserialize)]
pub(super) struct Format {
    pub source: Source,
    pub messages: Option<Vec<Message>>,
    pub response: Option<ModelResponse>,
    #[serde(default)]
    pub contextual_fallback: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct GroundingInput {
    pub messages: Option<Vec<Message>>,
    pub contextual_fallback: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct SelectionInput {
    pub texts: Vec<String>,
    pub context: SelectionContext,
}

#[derive(Debug, Deserialize)]
pub(super) struct Prepare {
    pub messages: Option<Vec<Message>>,
    pub latest_only: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct Merge {
    pub masked: Vec<String>,
    pub texts: Vec<String>,
    pub scanned_slice: Option<(usize, usize)>,
    pub scanned_role_subset: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct MergeMessages {
    pub original: Option<Vec<Message>>,
    pub updated: Vec<Message>,
    pub indices: Option<Vec<usize>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Extract {
    pub response: Option<Response>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MaskMessages {
    pub messages: Vec<Message>,
    pub masked_texts: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct MaskResponse {
    pub response: ModelResponse,
    pub masked_texts: Vec<String>,
}

#[derive(Serialize)]
pub(super) struct Grounding {
    pub text: String,
    pub qualifier: Option<Qualifier>,
}

#[derive(Serialize)]
pub(super) struct Selection {
    pub filtered_messages: Option<Vec<Message>>,
    pub scanned_slice: Option<(usize, usize)>,
    pub scanned_role_subset: bool,
    pub skip_scan: bool,
}

#[derive(Serialize)]
pub(super) struct Prepared {
    pub payload_messages: Option<Vec<Message>>,
    pub original_messages: Option<Vec<Message>>,
    pub target_indices: Option<Vec<usize>>,
}

impl From<QualifiedTextBlock> for Grounding {
    fn from(value: QualifiedTextBlock) -> Self {
        Self {
            text: value.text,
            qualifier: value.qualifier,
        }
    }
}
impl From<MessageSelection<Message>> for Selection {
    fn from(value: MessageSelection<Message>) -> Self {
        Self {
            filtered_messages: value.filtered_messages,
            scanned_slice: value.scanned_slice,
            scanned_role_subset: value.scanned_role_subset,
            skip_scan: value.skip_scan,
        }
    }
}
impl From<MessageFilterResult<Message>> for Prepared {
    fn from(value: MessageFilterResult<Message>) -> Self {
        Self {
            payload_messages: value.payload_messages,
            original_messages: value.original_messages,
            target_indices: value.target_indices,
        }
    }
}
