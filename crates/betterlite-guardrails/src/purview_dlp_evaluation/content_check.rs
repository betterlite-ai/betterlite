use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_prompts::arguments::json::{Json, Object};
use thiserror::Error;

use crate::guardrail_records::error::GuardrailRecordError;
use crate::guardrail_records::invocation::GuardrailInvocation;
use crate::guardrail_records::record::{
    GuardrailRecordContext, GuardrailRecordInput, GuardrailStatus,
};
use crate::purview_dlp_evaluation::GUARDRAIL_PROVIDER;
use crate::purview_dlp_evaluation::error_handling::{self, PurviewDlpHttpError};
use crate::purview_graph_client::PurviewGuardrailBase;
use crate::purview_graph_client::error::PurviewGraphError;
use crate::purview_graph_client::policy_action::should_block;
use crate::purview_graph_client::process_content::{ContentRequest, process_content};
use crate::purview_graph_client::scope_cache::compute_protection_scopes;
use crate::purview_graph_client::wire::{Activity, ProcessContentResponse, ProtectionScopeState};

pub struct ContentCheck<'request> {
    pub user_id: &'request str,
    pub text: &'request str,
    pub activity: Activity,
    pub request_data: &'request Object,
    pub block_on_violation: bool,
}

#[derive(Debug, Error)]
pub enum ContentCheckError {
    #[error(transparent)]
    Http(Box<PurviewDlpHttpError>),
    #[error("the Purview DLP guardrail record could not be written")]
    Record {
        #[source]
        source: Box<GuardrailRecordError>,
    },
}

// The record is a value the caller writes: the span sink is not Sync, so holding the context across the Graph calls would make the hook future not Send.
#[derive(Debug)]
#[must_use]
pub struct ContentOutcome {
    pub result: Result<ProcessContentResponse, PurviewDlpHttpError>,
    pub record: GuardrailRecordInput,
}

impl ContentOutcome {
    pub fn write_record(
        self,
        invocation: &mut GuardrailInvocation,
        context: &mut GuardrailRecordContext<'_>,
    ) -> Result<ProcessContentResponse, ContentCheckError> {
        invocation
            .record_own(context, self.record)
            .map_err(|source| ContentCheckError::Record {
                source: Box::new(source),
            })?;
        self.result
            .map_err(|error| ContentCheckError::Http(Box::new(error)))
    }
}

fn response_record(response: &ProcessContentResponse) -> GuardrailValue {
    let actions = response
        .policy_actions
        .iter()
        .map(|action| {
            GuardrailValue::Object(BTreeMap::from([
                (
                    "@odata.type".to_owned(),
                    GuardrailValue::Text(action.odata_type.clone()),
                ),
                (
                    "action".to_owned(),
                    GuardrailValue::Text(action.action.clone()),
                ),
                (
                    "restrictionAction".to_owned(),
                    GuardrailValue::Text(action.restriction_action.clone()),
                ),
            ]))
        })
        .collect();
    let mut fields = BTreeMap::from([("policyActions".to_owned(), GuardrailValue::Array(actions))]);
    let state = match response.protection_scope_state {
        Some(ProtectionScopeState::Modified) => Some("modified"),
        Some(ProtectionScopeState::NotModified) => Some("notModified"),
        Some(ProtectionScopeState::Other) | None => None,
    };
    if let Some(scope_state) = state {
        fields.insert(
            "protectionScopeState".to_owned(),
            GuardrailValue::Text(scope_state.to_owned()),
        );
    }
    GuardrailValue::Object(fields)
}

fn record_input(
    recorded: GuardrailValue,
    status: GuardrailStatus,
    started_at: SystemTime,
    finished_at: SystemTime,
) -> GuardrailRecordInput {
    let start_time = started_at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let end_time = finished_at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let duration = finished_at.duration_since(started_at).unwrap_or_default();
    GuardrailRecordInput {
        guardrail_response: recorded,
        guardrail_status: status,
        start_time: Some(start_time.as_secs_f64()),
        end_time: Some(end_time.as_secs_f64()),
        duration: Some(duration.as_secs_f64()),
        masked_entity_count: None,
        guardrail_provider: Some(GUARDRAIL_PROVIDER.to_owned()),
        event_type: None,
        tracing_detail: None,
    }
}

fn evaluation_failure(
    error: &PurviewGraphError,
    activity: Activity,
    block_on_violation: bool,
) -> Result<ProcessContentResponse, PurviewDlpHttpError> {
    if block_on_violation {
        return Err(error_handling::evaluation_http_error(error, activity));
    }
    tracing::warn!("Purview DLP: API/network error in logging-only mode (not re-raised): {error}");
    Ok(ProcessContentResponse {
        policy_actions: Vec::new(),
        protection_scope_state: None,
    })
}

async fn evaluate(
    base: &PurviewGuardrailBase,
    check: &ContentCheck<'_>,
) -> Result<ProcessContentResponse, PurviewGraphError> {
    let (etag, _protection_scopes) = compute_protection_scopes(base, check.user_id).await?;
    let correlation_id = check
        .request_data
        .get("litellm_call_id")
        .and_then(Json::as_str)
        .filter(|call_id| !call_id.is_empty());
    let request = ContentRequest {
        user_id: check.user_id,
        text: check.text,
        activity: check.activity,
        etag: etag.as_str(),
        correlation_id,
    };
    process_content(base, &request).await
}

pub async fn check_content(
    base: &PurviewGuardrailBase,
    check: &ContentCheck<'_>,
) -> ContentOutcome {
    let started_at = (base.clock)();
    let (result, status, recorded) = match evaluate(base, check).await {
        Ok(response) if should_block(&response) => {
            let answer = response_record(&response);
            let blocked = if check.block_on_violation {
                Err(error_handling::policy_block(check.activity))
            } else {
                Ok(response)
            };
            (blocked, GuardrailStatus::GuardrailIntervened, answer)
        }
        Ok(response) => {
            let answer = response_record(&response);
            (Ok(response), GuardrailStatus::Success, answer)
        }
        Err(error) => (
            evaluation_failure(&error, check.activity, check.block_on_violation),
            GuardrailStatus::GuardrailFailedToRespond,
            GuardrailValue::Object(BTreeMap::new()),
        ),
    };
    ContentOutcome {
        result,
        record: record_input(recorded, status, started_at, (base.clock)()),
    }
}

mod tests;
