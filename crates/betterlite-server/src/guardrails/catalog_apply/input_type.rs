use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;

pub(super) fn resolve(
    event_hook: Option<&LitellmParamsMode>,
    input_type: &str,
) -> GuardrailInputType {
    let post_call = matches!(event_hook, Some(LitellmParamsMode::One(hook)) if hook == "post_call");
    if input_type == "request" && post_call {
        return GuardrailInputType::Response;
    }
    if input_type == "response" {
        GuardrailInputType::Response
    } else {
        GuardrailInputType::Request
    }
}
