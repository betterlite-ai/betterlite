use serde_json::{Value, json};

use betterlite_auth::auth_policy_support::extract::extract_request_tool_names;
use betterlite_auth::auth_policy_support::guardrails::{
    GUARDRAIL_DENIAL, GuardrailCheckError, guardrail_modification_check,
};
use betterlite_auth::auth_policy_support::tools::{ToolCheckError, check_tools_allowlist};
use betterlite_oracle_helper_api::helper::HelperResult;

use super::fixture_support::extractors;
use super::{Input, team_obj, token_in};

pub(super) fn guardrail(input: &Input) -> HelperResult {
    let team = team_obj(input.team.as_ref())?;
    let outcome = guardrail_modification_check(&input.body, team.as_ref());
    Ok(guardrail_json(outcome))
}

fn guardrail_json(outcome: Result<(), GuardrailCheckError>) -> Value {
    match outcome {
        Ok(()) => json!({ "outcome": "allowed" }),
        Err(GuardrailCheckError::Denied) => json!({
            "outcome": "denied",
            "status": 403,
            "detail": { "error": GUARDRAIL_DENIAL },
        }),
        Err(GuardrailCheckError::Metadata(error)) => {
            json!({ "outcome": "error", "message": error.to_string() })
        }
    }
}

pub(super) fn tools(input: &Input) -> HelperResult {
    let token = token_in(input.token.as_ref())?;
    let team = team_obj(input.team.as_ref())?;
    let extractors = extractors()?;
    let Ok(names) = extract_request_tool_names(&input.route, &input.body, &extractors) else {
        return Ok(json!({ "outcome": "not_iterable" }));
    };
    Ok(
        match check_tools_allowlist(
            &input.body,
            token.as_ref(),
            team.as_ref(),
            &input.route,
            &extractors,
        ) {
            Ok(()) => json!({ "outcome": "allowed", "names": names }),
            Err(ToolCheckError::Denied(exception)) => json!({
                "outcome": "denied",
                "names": names,
                "error_type": exception.error_type,
                "param": exception.param,
                "code": exception.code,
            }),
            Err(ToolCheckError::Extraction(_)) => json!({ "outcome": "not_iterable" }),
        },
    )
}
