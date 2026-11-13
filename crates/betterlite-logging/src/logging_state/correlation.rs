use betterlite_redact::secret::Redactor;
use serde::{Deserialize, Serialize};
use unicode_properties::{GeneralCategoryGroup, UnicodeGeneralCategory as _};

use super::LoggingState;

pub const MAXIMUM_CORRELATION_ID_LENGTH: usize = 256;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct CorrelationContext {
    pub trace_id: String,
    pub session_id: String,
}

// LITELLM_DISABLE_REDACT_SECRETS=true resolves to Disabled
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SecretRedaction {
    #[default]
    Enabled,
    Disabled,
}

// Mirrors LiteLLM's constructor default: supports_correlation_logging is True unless the caller passes False
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct CorrelationSettings {
    pub supports_correlation_logging: bool,
    pub request_correlation_in_logs: bool,
    pub secret_redaction: SecretRedaction,
}

impl Default for CorrelationSettings {
    fn default() -> Self {
        Self {
            supports_correlation_logging: true,
            request_correlation_in_logs: false,
            secret_redaction: SecretRedaction::Enabled,
        }
    }
}

pub struct CorrelationSetup<'context> {
    pub context: &'context mut CorrelationContext,
    pub settings: CorrelationSettings,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorrelationOwnership {
    pub pre_call: CorrelationContext,
    pub own: CorrelationContext,
}

// Python's str.isprintable drops separators and other control, format, private and unassigned characters
#[must_use]
pub fn sanitize_correlation_id(value: &str, redactor: Option<&Redactor>) -> String {
    let printable: String = value
        .chars()
        .filter(|character| {
            *character == ' '
                || !matches!(
                    character.general_category_group(),
                    GeneralCategoryGroup::Separator | GeneralCategoryGroup::Other
                )
        })
        .collect();
    let redacted = match redactor {
        Some(active) => active.redact_string(&printable),
        None => printable,
    };
    redacted
        .chars()
        .take(MAXIMUM_CORRELATION_ID_LENGTH)
        .collect()
}

pub(super) fn apply_correlation(
    correlation: &mut CorrelationSetup<'_>,
    trace_id: &str,
    session_id: &str,
    redactor: &Redactor,
) -> CorrelationOwnership {
    let pre_call = correlation.context.clone();
    if correlation.settings.supports_correlation_logging
        && correlation.settings.request_correlation_in_logs
    {
        let active =
            (correlation.settings.secret_redaction == SecretRedaction::Enabled).then_some(redactor);
        correlation.context.trace_id = sanitize_correlation_id(trace_id, active);
        correlation.context.session_id = sanitize_correlation_id(session_id, active);
    }
    let own = correlation.context.clone();
    CorrelationOwnership { pre_call, own }
}

// Deliberately unguarded: every task that shared this call needs its own restore to take effect
pub fn restore_correlation_context(state: &LoggingState, context: &mut CorrelationContext) {
    context
        .trace_id
        .clone_from(&state.correlation.pre_call.trace_id);
    context
        .session_id
        .clone_from(&state.correlation.pre_call.session_id);
}

// Late cleanup must not stomp ids an unrelated active call has since set
pub fn restore_correlation_guarded(state: &LoggingState, context: &mut CorrelationContext) {
    if *context == state.correlation.own {
        restore_correlation_context(state, context);
    }
}
