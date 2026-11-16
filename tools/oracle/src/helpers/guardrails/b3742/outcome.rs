use std::collections::BTreeSet;
use std::sync::Arc;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::agent_365::enforcement::error::EnforcementError;
use betterlite_guardrails::agent_365::enforcement::pre_mcp_call_hook;
use betterlite_guardrails::agent_365_tokens::Agent365Guardrail;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use super::poster::{ScriptedReplies, guardrail};
use super::{Data, EnforcementArgs, Invocation, METADATA_KEY, RECORDS_KEY};
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::HelperResult;

// What the caller observes when the hook raises: the HTTP status and the error detail
fn failure_outcome(failure: &EnforcementError) -> Result<Value, Box<HelperError>> {
    match failure {
        EnforcementError::Guardrail(GuardrailFailure::HttpObject { status, detail }) => {
            let detail = serde_json::to_value(detail).or_value_error()?;
            Ok(json!({"kind": "rejected", "status_code": status, "detail": detail}))
        }
        EnforcementError::Guardrail(GuardrailFailure::Http { status, detail }) => {
            Ok(json!({"kind": "rejected", "status_code": status, "detail": detail}))
        }
        other => Err(format!("the Agent 365 hook failed unexpectedly: {other}")).or_value_error(),
    }
}

fn records_of(data: &Data) -> Result<Value, Box<HelperError>> {
    let Some(GuardrailValue::Object(metadata)) = data.get(METADATA_KEY) else {
        return Ok(Value::Array(Vec::new()));
    };
    match metadata.get(RECORDS_KEY) {
        Some(records) => serde_json::to_value(records).or_value_error(),
        None => Ok(Value::Array(Vec::new())),
    }
}

async fn run_invocation(
    guardrail: &Agent365Guardrail,
    invocation: &Invocation,
    key_alias: Option<&str>,
) -> Result<(Value, Value), Box<HelperError>> {
    let mut data = invocation.data.clone();
    let suppressed = BTreeSet::new();
    let called = pre_mcp_call_hook(
        guardrail,
        &mut data,
        &invocation.call_type,
        key_alias,
        &suppressed,
    )
    .await;
    let outcome = match called {
        Ok(()) => json!({"kind": "allowed"}),
        Err(failure) => failure_outcome(&failure)?,
    };
    Ok((outcome, records_of(&data)?))
}

pub(super) fn enforcement_result(args: &EnforcementArgs) -> HelperResult {
    let poster = Arc::new(ScriptedReplies::new(&args.replies, args.repeat_last_reply)?);
    let guardrail = guardrail(Arc::clone(&poster), args.unreachable_fallback)?;
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .or_value_error()?;
    let mut outcomes = Vec::new();
    let mut records = Vec::new();
    for invocation in &args.invocations {
        let (outcome, written) = runtime.block_on(run_invocation(
            &guardrail,
            invocation,
            args.key_alias.as_deref(),
        ))?;
        outcomes.push(outcome);
        records.push(written);
    }
    Ok(json!({
        "outcomes": outcomes,
        "records": records,
        "requests": poster.recorded(),
    }))
}
