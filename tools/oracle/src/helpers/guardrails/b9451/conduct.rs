use betterlite_config::sections::guardrail_params::base::UnreachableFallback;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_guardrails::third_party_hooks_a::conduct;
use betterlite_guardrails::third_party_init_a::conduct::ConductArgs;
use serde::Deserialize;
use serde_json::{Value, json};

use super::scripted::outcome_view;

#[derive(Debug, Deserialize)]
pub(crate) struct Case {
    name: String,
}

pub(super) fn run(case: &Case) -> Value {
    let args = ConductArgs {
        name: case.name.clone(),
        api_url: None,
        agent_token: None,
        workspace_id: None,
        tool_name: "llm_call".to_owned(),
        unreachable_fallback: UnreachableFallback::FailClosed,
        timeout: 8.0,
        event_hook: LitellmParamsMode::One("pre_call".to_owned()),
        default_on: false,
    };
    let outcome = conduct::build(&args).map(|_guardrail| json!({"built": true}));
    outcome_view(&[], &[], &[], &outcome)
}
