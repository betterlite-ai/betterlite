use std::collections::BTreeSet;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue, InvalidHeaderValue};
use secrecy::{ExposeSecret as _, SecretString};

use crate::agent_365::enforcement::error::EnforcementError;
use crate::agent_365::enforcement::handlers::EvaluationReply;
use crate::agent_365::enforcement::record::VerdictRecord;
use crate::agent_365::enforcement_types::Verdict;
use crate::agent_365_tokens::error::{Agent365TokenError, Agent365TokenExchangeError};
use crate::agent_365_tokens::exchange::entra_assertion;
use crate::agent_365_tokens::payload::{Agent365PayloadError, build_evaluate_payload};
use crate::agent_365_tokens::poster::PostReply;
use crate::agent_365_tokens::{Agent365Guardrail, EVALUATE_URL};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_metadata::RequestData;
use crate::guardrail_selection::gate::GuardrailSelection as _;

pub mod error;
mod handlers;
mod record;
mod refusals;
mod tests;

const MCP_CALL_TYPES: [&str; 2] = ["mcp_call", "call_mcp_tool"];
const MISSING_BEARER_REASON: &str = "the caller did not present an Entra bearer token; the Agent 365 guardrail authorizes tool calls On-Behalf-Of the signed-in user";
const JSON_CONTENT_TYPE: &str = "application/json";
const STATUS_UNAUTHORIZED: u16 = 401;

// handle_evaluate_error says whether the verdict still has to be enforced
enum Step {
    Continue,
    Done,
}

// LiteLLM's async_pre_call_hook for the pre_mcp_call event: `suppressed` is what an auto router turned off
pub async fn pre_mcp_call_hook(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    call_type: &str,
    key_alias: Option<&str>,
    suppressed: &BTreeSet<String>,
) -> Result<(), EnforcementError> {
    if !runs_for(guardrail, data, call_type, suppressed)? {
        return Ok(());
    }
    let tool_name = tool_name(data)?;
    let Some(assertion) = resolve_assertion(data) else {
        return refusals::refuse(
            guardrail,
            data,
            &refusals::Refusal::CallerFault {
                status: STATUS_UNAUTHORIZED,
                tool_name: tool_name.as_str(),
                reason: MISSING_BEARER_REASON,
            },
        )
        .map_err(EnforcementError::from);
    };
    let Some(token) = obo_token(guardrail, data, &tool_name, assertion.expose_secret()).await?
    else {
        return Ok(());
    };
    let Some((reply, latency)) = evaluate(guardrail, data, &tool_name, &token, key_alias).await?
    else {
        return Ok(());
    };
    let evaluation = EvaluationReply {
        tool_name: &tool_name,
        assertion: assertion.expose_secret(),
        reply: &reply,
        latency,
    };
    handlers::enforce_reply(guardrail, data, &evaluation).map_err(EnforcementError::from)
}

fn runs_for(
    guardrail: &Agent365Guardrail,
    data: &RequestData,
    call_type: &str,
    suppressed: &BTreeSet<String>,
) -> Result<bool, GuardrailFailure> {
    if !MCP_CALL_TYPES.contains(&call_type) {
        return Ok(false);
    }
    if !data.contains_key("mcp_tool_name") {
        return Ok(false);
    }
    let runs =
        guardrail
            .base
            .should_run_guardrail(data, GuardrailEventHooks::PreMcpCall, suppressed)?;
    Ok(runs)
}

fn resolve_assertion(data: &RequestData) -> Option<SecretString> {
    let value = data.get("incoming_bearer_token")?;
    let assertion = entra_assertion(value)?;
    Some(SecretString::from(assertion))
}

// Ok(None) means a fail_open fallback already allowed the call
async fn obo_token(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    assertion: &str,
) -> Result<Option<SecretString>, GuardrailFailure> {
    match guardrail.get_obo_token(assertion).await {
        Ok(token) => Ok(Some(token)),
        Err(error) => {
            token_failure(guardrail, data, tool_name, error)?;
            Ok(None)
        }
    }
}

fn token_failure(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    error: Agent365TokenError,
) -> Result<(), GuardrailFailure> {
    let reason = token_failure_reason(&error);
    match error {
        Agent365TokenError::Exchange(exchange) if !exchange.gateway_owned() => refusals::refuse(
            guardrail,
            data,
            &refusals::Refusal::CallerFault {
                status: STATUS_UNAUTHORIZED,
                tool_name,
                reason: &reason,
            },
        ),
        Agent365TokenError::Throttled { .. } => refusals::refuse(
            guardrail,
            data,
            &refusals::Refusal::Throttled {
                tool_name,
                reason: &reason,
                latency: None,
            },
        ),
        Agent365TokenError::Exchange(_)
        | Agent365TokenError::NonJsonBody { .. }
        | Agent365TokenError::NonObjectBody { .. }
        | Agent365TokenError::MissingAccessToken
        | Agent365TokenError::BadAccessToken
        | Agent365TokenError::HttpStatus { .. }
        | Agent365TokenError::TokenExpiry
        | Agent365TokenError::Poster(_) => handlers::unavailable(
            guardrail,
            data,
            tool_name,
            &VerdictRecord {
                verdict: Verdict::Unavailable,
                defender_status: None,
                correlation_id: None,
                latency: None,
                reason: Some(&reason),
            },
        ),
    }
}

fn token_failure_reason(error: &Agent365TokenError) -> String {
    match error {
        Agent365TokenError::Exchange(exchange) => exchange_reason(exchange),
        Agent365TokenError::Throttled { status } => {
            format!("the Entra token endpoint returned HTTP {status}")
        }
        Agent365TokenError::Poster(_) | Agent365TokenError::HttpStatus { .. } => {
            format!("the Entra token endpoint could not be reached ({error})")
        }
        Agent365TokenError::NonJsonBody { .. }
        | Agent365TokenError::NonObjectBody { .. }
        | Agent365TokenError::MissingAccessToken
        | Agent365TokenError::BadAccessToken
        | Agent365TokenError::TokenExpiry => error.to_string(),
    }
}

fn exchange_reason(error: &Agent365TokenExchangeError) -> String {
    if error.gateway_owned() {
        return format!(
            "Entra rejected the gateway's own Agent 365 credentials ({}); check the guardrail's client_id and client_secret",
            error.error_code
        );
    }
    format!(
        "the Entra On-Behalf-Of token exchange was rejected ({})",
        error.error_code
    )
}

// Ok(None) means a fail_open fallback already allowed the call
async fn evaluate(
    guardrail: &Agent365Guardrail,
    data: &mut RequestData,
    tool_name: &str,
    token: &SecretString,
    key_alias: Option<&str>,
) -> Result<Option<(PostReply, Duration)>, EnforcementError> {
    let body = evaluate_body(data, key_alias)?;
    // LiteLLM's httpx raises LocalProtocolError for a bearer value no header can carry, so both failures are unreachable
    let reason = match evaluation_headers(token) {
        Ok(headers) => {
            let started = (guardrail.clock)();
            match guardrail
                .post_allowing_error_status(EVALUATE_URL, &headers, &body)
                .await
            {
                Ok(reply) => {
                    let finished = (guardrail.clock)();
                    let latency = finished.duration_since(started).unwrap_or_default();
                    return Ok(Some((reply, latency)));
                }
                Err(error) => format!("the Agent 365 endpoint could not be reached ({error})"),
            }
        }
        Err(error) => format!(
            "the Agent 365 endpoint could not be reached (the OBO token is not a valid header value: {error})"
        ),
    };
    handlers::unavailable(
        guardrail,
        data,
        tool_name,
        &VerdictRecord {
            verdict: Verdict::Unavailable,
            defender_status: None,
            correlation_id: None,
            latency: None,
            reason: Some(&reason),
        },
    )?;
    Ok(None)
}

fn evaluate_body(data: &RequestData, key_alias: Option<&str>) -> Result<Vec<u8>, EnforcementError> {
    let payload = match build_evaluate_payload(data, key_alias) {
        Ok(payload) => payload,
        Err(error) => {
            return Err(EnforcementError::Guardrail(GuardrailFailure::Http {
                status: 400,
                detail: error.to_string(),
            }));
        }
    };
    serde_json::to_vec(&payload).map_err(|source| EnforcementError::PayloadEncoding { source })
}

// The bearer value is marked sensitive so the header map never prints the OBO token
fn evaluation_headers(token: &SecretString) -> Result<HeaderMap, InvalidHeaderValue> {
    let value = SecretString::from(format!("Bearer {}", token.expose_secret()));
    let mut authorization = HeaderValue::from_str(value.expose_secret())?;
    authorization.set_sensitive(true);
    let mut headers = HeaderMap::new();
    let _content_type = headers.insert(CONTENT_TYPE, HeaderValue::from_static(JSON_CONTENT_TYPE));
    let _authorization = headers.insert(AUTHORIZATION, authorization);
    Ok(headers)
}

fn tool_name(data: &RequestData) -> Result<String, GuardrailFailure> {
    match data.get("mcp_tool_name") {
        None | Some(GuardrailValue::Null) => Ok(String::new()),
        Some(GuardrailValue::Text(name)) => Ok(name.clone()),
        Some(_) => {
            let error = Agent365PayloadError::TextField {
                field: "mcp_tool_name",
            };
            Err(GuardrailFailure::Http {
                status: 400,
                detail: error.to_string(),
            })
        }
    }
}
