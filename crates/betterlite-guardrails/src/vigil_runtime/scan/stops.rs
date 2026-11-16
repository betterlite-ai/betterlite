use crate::guardrail_base::failure::GuardrailFailure;
use crate::vigil_guard::error::VigilGuardError;
use crate::vigil_guard::output::ToolCall;

pub(super) enum ScanStop {
    Failed(VigilGuardError),
    Blocked(GuardrailFailure),
    Unrecognized,
}

// A text-stage stop carries the partial result its fail_open answer needs
pub(super) enum TextStop {
    Blocked(GuardrailFailure),
    Interrupted {
        error: VigilGuardError,
        partial: Vec<String>,
    },
    Unrecognized {
        partial: Vec<String>,
    },
}

// Every analyze failure is either a backend failure the fallback policy handles or a local error it must not swallow
pub(super) enum FailureKind {
    Backend,
    Local,
}

pub(super) const fn failure_kind(error: &VigilGuardError) -> FailureKind {
    match error {
        VigilGuardError::InvalidHeader { .. }
        | VigilGuardError::Transport { .. }
        | VigilGuardError::ResponseJson { .. } => FailureKind::Backend,
        VigilGuardError::MissingConfig { .. }
        | VigilGuardError::ToolCallIndex { .. }
        | VigilGuardError::InvalidToolFunction { .. }
        | VigilGuardError::Blocked { .. }
        | VigilGuardError::BackendUnavailable => FailureKind::Local,
    }
}

#[derive(Clone, Copy)]
pub(super) struct Partial<'partial> {
    pub(super) texts: &'partial [String],
    pub(super) tool_calls: Option<&'partial [ToolCall]>,
}

pub(super) fn partial_texts(processed: &[String], texts: &[String], index: usize) -> Vec<String> {
    let mut partial = processed.to_vec();
    if let Some(remaining) = texts.get(index..) {
        partial.extend_from_slice(remaining);
    }
    partial
}
