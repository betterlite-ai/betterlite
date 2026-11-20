use betterlite_server::guardrails::anthropic_translation::scan_targets::{
    InputWriteBackTarget, MessageTextTarget,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub(crate) struct ToolUseTargetRequest {
    pub(crate) msg_idx: usize,
    pub(crate) content_idx: usize,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum MessageTargetRequest {
    MessageContent {
        msg_idx: usize,
    },
    ContentBlockText {
        msg_idx: usize,
        content_idx: usize,
    },
    ToolResultString {
        msg_idx: usize,
        content_idx: usize,
    },
    ToolResultBlockText {
        msg_idx: usize,
        content_idx: usize,
        block_idx: usize,
    },
}

impl From<&MessageTargetRequest> for MessageTextTarget {
    fn from(request: &MessageTargetRequest) -> Self {
        match request {
            MessageTargetRequest::MessageContent { msg_idx } => {
                Self::MessageContent { msg_idx: *msg_idx }
            }
            MessageTargetRequest::ContentBlockText {
                msg_idx,
                content_idx,
            } => Self::ContentBlockText {
                msg_idx: *msg_idx,
                content_idx: *content_idx,
            },
            MessageTargetRequest::ToolResultString {
                msg_idx,
                content_idx,
            } => Self::ToolResultString {
                msg_idx: *msg_idx,
                content_idx: *content_idx,
            },
            MessageTargetRequest::ToolResultBlockText {
                msg_idx,
                content_idx,
                block_idx,
            } => Self::ToolResultBlockText {
                msg_idx: *msg_idx,
                content_idx: *content_idx,
                block_idx: *block_idx,
            },
        }
    }
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum InputTargetRequest {
    SystemString,
    SystemBlockText {
        block_idx: usize,
    },
    MessageContent {
        msg_idx: usize,
    },
    ContentBlockText {
        msg_idx: usize,
        content_idx: usize,
    },
    ToolResultString {
        msg_idx: usize,
        content_idx: usize,
    },
    ToolResultBlockText {
        msg_idx: usize,
        content_idx: usize,
        block_idx: usize,
    },
}

impl From<&InputTargetRequest> for InputWriteBackTarget {
    fn from(request: &InputTargetRequest) -> Self {
        match request {
            InputTargetRequest::SystemString => Self::SystemString,
            InputTargetRequest::SystemBlockText { block_idx } => Self::SystemBlockText {
                block_idx: *block_idx,
            },
            InputTargetRequest::MessageContent { msg_idx } => {
                Self::Message(MessageTextTarget::MessageContent { msg_idx: *msg_idx })
            }
            InputTargetRequest::ContentBlockText {
                msg_idx,
                content_idx,
            } => Self::Message(MessageTextTarget::ContentBlockText {
                msg_idx: *msg_idx,
                content_idx: *content_idx,
            }),
            InputTargetRequest::ToolResultString {
                msg_idx,
                content_idx,
            } => Self::Message(MessageTextTarget::ToolResultString {
                msg_idx: *msg_idx,
                content_idx: *content_idx,
            }),
            InputTargetRequest::ToolResultBlockText {
                msg_idx,
                content_idx,
                block_idx,
            } => Self::Message(MessageTextTarget::ToolResultBlockText {
                msg_idx: *msg_idx,
                content_idx: *content_idx,
                block_idx: *block_idx,
            }),
        }
    }
}
