use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use indexmap::IndexMap;
use serde::Serialize;

use super::record::write_record;
use super::{ChecksError, ChecksInvocation, RecordOutcome, RecordTimes};
use crate::bedrock_checks::block;
use crate::bedrock_checks::sanitize;
use crate::bedrock_checks::violations::{self, ChecksThresholds};
use crate::bedrock_records::{ChecksResponse, Response};
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_records::record::GuardrailStatus;

const UNEXPECTED_SHAPE: &str =
    "Bedrock InvokeGuardrailChecks returned an unexpected response shape";

pub(super) fn decide(
    invocation: &mut ChecksInvocation<'_>,
    event_type: GuardrailEventHooks,
    times: RecordTimes,
    body: &[u8],
) -> Result<Response, GuardrailFailure> {
    let response = match serde_json::from_slice::<ChecksResponse>(body) {
        Ok(response) => response,
        Err(source) => return Err(unparseable(invocation, event_type, times, &source)),
    };
    let empty = BTreeMap::new();
    let configured = invocation.guardrail.checks.as_ref().unwrap_or(&empty);
    let thresholds = ChecksThresholds {
        content_filter: invocation.guardrail.content_filter_threshold,
        prompt_attack: invocation.guardrail.prompt_attack_threshold,
        sensitive_information: invocation.guardrail.pii_confidence_threshold,
    };
    let found = violations::collect(Some(&response), configured, thresholds);
    let sanitized = sanitize::for_logging(&response);
    let logged = to_guardrail_value(&sanitized)?;
    let status = violations::status(!found.is_empty());
    let tracing_detail = (!found.is_empty()).then(|| violations::tracing_detail(&found));
    write_record(
        invocation,
        event_type,
        times,
        RecordOutcome {
            response: logged,
            status,
            tracing_detail,
        },
    );
    if found.is_empty() {
        return Ok(Response::default());
    }
    Err(block::exception(
        invocation.guardrail,
        invocation.base,
        &found,
        &*invocation.request_data,
    ))
}

fn unparseable(
    invocation: &mut ChecksInvocation<'_>,
    event_type: GuardrailEventHooks,
    times: RecordTimes,
    source: &serde_json::Error,
) -> GuardrailFailure {
    let mut fields = BTreeMap::new();
    fields.insert("error".to_owned(), GuardrailValue::Text(source.to_string()));
    write_record(
        invocation,
        event_type,
        times,
        RecordOutcome {
            response: GuardrailValue::Object(fields),
            status: GuardrailStatus::GuardrailFailedToRespond,
            tracing_detail: None,
        },
    );
    let mut detail = IndexMap::new();
    detail.insert(
        "error".to_owned(),
        GuardrailValue::Text(UNEXPECTED_SHAPE.to_owned()),
    );
    GuardrailFailure::HttpObject {
        status: 500,
        detail,
    }
}

fn to_guardrail_value<T>(value: &T) -> Result<GuardrailValue, ChecksError>
where
    T: Serialize,
{
    let encoded = serde_json::to_value(value).map_err(ChecksError::Encode)?;
    serde_json::from_value(encoded).map_err(ChecksError::Encode)
}
