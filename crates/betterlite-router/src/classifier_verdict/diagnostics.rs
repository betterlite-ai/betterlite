use std::fmt;

use betterlite_logging::spend_bodies::redaction::should_redact_message_logging;

use super::call::{RequestKwargs, dynamic_params};
use super::error::ClassifierVerdictError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RejectedReply {
    Withheld,
    Raw(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectionDiagnostic {
    pub problems: Vec<String>,
    pub reply: RejectedReply,
}

impl fmt::Display for RejectionDiagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let problems = self.problems.join("; ");
        match &self.reply {
            RejectedReply::Withheld => write!(
                formatter,
                "ComplexityRouter: classifier verdict rejected ({problems}); raw reply withheld (message logging is off)"
            ),
            RejectedReply::Raw(content) => write!(
                formatter,
                "ComplexityRouter: classifier verdict rejected ({problems}); raw reply: {content:?}"
            ),
        }
    }
}

#[must_use]
pub fn verdict_problems(error: &ClassifierVerdictError) -> Vec<String> {
    match error {
        ClassifierVerdictError::ReplyTooLarge { bytes, limit } => vec![format!(
            "the reply is {bytes} bytes, over the {limit}-byte parsing limit"
        )],
        ClassifierVerdictError::ReplyNotText { .. } => {
            vec!["the reply is not valid UTF-8".to_owned()]
        }
        ClassifierVerdictError::NoVerdictObject => {
            vec!["the reply contains no complete JSON object".to_owned()]
        }
        ClassifierVerdictError::VerdictSchema { source } => vec![format!("tier: {source}")],
        ClassifierVerdictError::UnknownTier { .. }
        | ClassifierVerdictError::RubricExamples { .. }
        | ClassifierVerdictError::MissingLabel { .. }
        | ClassifierVerdictError::TierCriterion { .. }
        | ClassifierVerdictError::OperatorSectionBlank { .. }
        | ClassifierVerdictError::OperatorSectionTooLong { .. }
        | ClassifierVerdictError::RequestKwargs { .. }
        | ClassifierVerdictError::CallbackMetadata { .. } => Vec::new(),
    }
}

#[must_use]
pub fn classifier_reply_is_private(
    request_kwargs: Option<&RequestKwargs>,
    global_turn_off: bool,
) -> bool {
    let Ok(dynamic) = dynamic_params(request_kwargs) else {
        return true;
    };
    should_redact_message_logging(request_kwargs, dynamic.as_ref(), global_turn_off)
}

#[must_use]
pub fn rejection_diagnostic(
    error: &ClassifierVerdictError,
    content: &str,
    request_kwargs: Option<&RequestKwargs>,
    global_turn_off: bool,
) -> Option<RejectionDiagnostic> {
    if matches!(error, ClassifierVerdictError::UnknownTier { .. }) {
        return None;
    }
    let reply = if classifier_reply_is_private(request_kwargs, global_turn_off) {
        RejectedReply::Withheld
    } else {
        RejectedReply::Raw(content.to_owned())
    };
    Some(RejectionDiagnostic {
        problems: verdict_problems(error),
        reply,
    })
}
