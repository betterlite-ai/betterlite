use super::InputType;

pub(super) struct CallIdentity {
    pub input_type: InputType,
    pub litellm_call_id: Option<String>,
    pub litellm_trace_id: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) enum Outcome<'call> {
    Rejected,
    Bypassed(&'call CallIdentity),
}

pub(super) fn failure(
    guardrail_name: &str,
    api_base: &str,
    failure_category: &str,
    status: Option<&str>,
    outcome: Outcome<'_>,
) {
    match outcome {
        Outcome::Rejected => tracing::error!(
            guardrail_name,
            failure_category,
            status = status.unwrap_or("n/a"),
            "DeepKeep guardrail call failed",
        ),
        Outcome::Bypassed(identity) => {
            let input_type = match identity.input_type {
                InputType::Request => "request",
                InputType::Response => "response",
            };
            tracing::error!(
                guardrail_name,
                api_base,
                failure_category,
                status = status.unwrap_or("n/a"),
                input_type,
                litellm_call_id = identity.litellm_call_id.as_deref().unwrap_or("n/a"),
                litellm_trace_id = identity.litellm_trace_id.as_deref().unwrap_or("n/a"),
                "DeepKeep guardrail unreachable; evaluation bypassed because unreachable_fallback is fail_open",
            );
        }
    }
}
