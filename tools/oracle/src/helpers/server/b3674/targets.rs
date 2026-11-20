use betterlite_oracle_helper_api::error::HelperError;
use betterlite_server::guardrails::anthropic_translation::scan_targets::{
    InputWriteBackTarget, MessageTextTarget, ToolUseInputTarget,
};
use serde::Deserialize;

macro_rules! value_error {
    ($message:literal) => {
        Box::new(HelperError {
            class: "ValueError".to_owned(),
            status: None,
            error_type: Some(None),
            param: Some(None),
            code: Some(None),
            message: $message.to_owned(),
        })
    };
}

#[derive(Default, Deserialize)]
pub(crate) struct ToolUseTargetRequest {
    pub(crate) msg_idx: usize,
    pub(crate) content_idx: usize,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum TargetRequest {
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

impl From<&TargetRequest> for InputWriteBackTarget {
    fn from(request: &TargetRequest) -> Self {
        match request {
            TargetRequest::SystemString => Self::SystemString,
            TargetRequest::SystemBlockText { block_idx } => Self::SystemBlockText {
                block_idx: *block_idx,
            },
            TargetRequest::MessageContent { msg_idx } => {
                Self::Message(MessageTextTarget::MessageContent { msg_idx: *msg_idx })
            }
            TargetRequest::ContentBlockText {
                msg_idx,
                content_idx,
            } => Self::Message(MessageTextTarget::ContentBlockText {
                msg_idx: *msg_idx,
                content_idx: *content_idx,
            }),
            TargetRequest::ToolResultString {
                msg_idx,
                content_idx,
            } => Self::Message(MessageTextTarget::ToolResultString {
                msg_idx: *msg_idx,
                content_idx: *content_idx,
            }),
            TargetRequest::ToolResultBlockText {
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

pub(crate) fn message_target(
    request: Option<&TargetRequest>,
) -> Result<MessageTextTarget, Box<HelperError>> {
    match request.map(InputWriteBackTarget::from) {
        Some(InputWriteBackTarget::Message(target)) => Ok(target),
        Some(InputWriteBackTarget::SystemString | InputWriteBackTarget::SystemBlockText { .. })
        | None => Err(value_error!("message scenario needs a message target")),
    }
}

impl From<&ToolUseTargetRequest> for ToolUseInputTarget {
    fn from(request: &ToolUseTargetRequest) -> Self {
        Self {
            msg_idx: request.msg_idx,
            content_idx: request.content_idx,
        }
    }
}
