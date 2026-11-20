use thiserror::Error;

use betterlite_guardrails::guardrail_message_scope::unappliable_rewrite::UnappliableRequestRewrite;

use crate::guardrails::anthropic_translation::scan_targets::MessageTextTarget;

// LiteLLM raises IndexError, KeyError or AttributeError; the named error carries the target instead.
#[derive(Debug, PartialEq, Eq, Error)]
pub enum WriteBackError {
    #[error("the guardrail rewrite targets a message block the request no longer has: {target:?}")]
    TargetMissing { target: MessageTextTarget },
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ApplyError {
    #[error(transparent)]
    Unappliable(#[from] UnappliableRequestRewrite),
    #[error("the request has no message at index {msg_idx} to write the rewritten tool call into")]
    MessageMissing { msg_idx: usize },
}
