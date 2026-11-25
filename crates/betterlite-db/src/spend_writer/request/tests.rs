#![cfg(test)]

use betterlite_models::model_registration::JsonObject;
use betterlite_models::tool_spend::ToolUsageTransaction;
use rust_decimal::Decimal;

use crate::key_user_writes::hash::hash_token;
use crate::redis_spend::log_row::SpendLogValue;
use crate::request_usage::queues::UsageQueue;
use crate::spend_log_storage::batch_claim::{BatchChargeRow, SpendLogAttachment};
use crate::spend_log_storage::rows::SpendLogQueue;
use crate::spend_queues::entity::SpendAmount;
use crate::spend_writer::claim::{InsertOutcome, ScriptedClaim};
use crate::spend_writer::settings::SpendWriterSettings;
use crate::spend_writer::support::{
    NoPrices, PayloadScript, RecordedFailure, RecordingAdmission, RecordingFailures,
    ScriptedPayload,
};
use crate::spend_writer::{DbSpendUpdateWriter, SpendWriterOperation, SpendWriterQueues};
use crate::tool_discovery::ToolDiscoveryQueue;

use super::SpendUpdateRequest;

const PAYLOAD: &str = r#"{"user":"u1","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","prompt_tokens":3,"completion_tokens":4,"request_id":"req-1","total_tokens":7,"startTime":"2025-01-02T03:04:05Z","endTime":"2025-01-02T03:04:06Z","request_tags":"[]","metadata":"{}","status":"success","spend":0.25}"#;
const BATCH_PAYLOAD: &str = r#"{"user":"u1","api_key":"k1","model":"gpt-4o","custom_llm_provider":"openai","model_group":"gpt-4o","prompt_tokens":3,"completion_tokens":4,"request_id":"batch-1","total_tokens":7,"startTime":"2025-01-02T03:04:05Z","call_type":"aretrieve_batch","status":"success","spend":0.25}"#;

struct Harness {
    claim: ScriptedClaim,
    log_queue: SpendLogQueue,
    tool_usage: UsageQueue<ToolUsageTransaction>,
    discovery: ToolDiscoveryQueue,
    failures: RecordingFailures,
    admission: RecordingAdmission,
}

impl Harness {
    fn new(claim: ScriptedClaim) -> Self {
        Self::with_admission(claim, RecordingAdmission::admitting())
    }

    fn with_admission(claim: ScriptedClaim, admission: RecordingAdmission) -> Self {
        Self {
            claim,
            log_queue: SpendLogQueue::new(usize::MAX),
            tool_usage: UsageQueue::new(10).expect("the usage queue"),
            discovery: ToolDiscoveryQueue::new(10).expect("the discovery queue"),
            failures: RecordingFailures::new(),
            admission,
        }
    }

    fn writer(&self, settings: SpendWriterSettings) -> DbSpendUpdateWriter<'_, ScriptedClaim> {
        DbSpendUpdateWriter {
            settings,
            queues: SpendWriterQueues {
                entity: None,
                daily_user: None,
                daily_end_user: None,
                daily_agent: None,
                daily_team: None,
                daily_org: None,
                daily_tag: None,
                tool_usage: Some(&self.tool_usage),
                model_usage: None,
                discovery: Some(&self.discovery),
                turns: None,
            },
            attachment: Some(SpendLogAttachment {
                table: &self.claim,
                queue: &self.log_queue,
            }),
            failures: &self.failures,
            admission: &self.admission,
        }
    }
}

fn sample_request<'call>() -> SpendUpdateRequest<'call, NoPrices> {
    SpendUpdateRequest {
        response_cost: Some(SpendAmount(Decimal::new(5, 1))),
        token: Some("sk-secret"),
        user_id: Some("u1"),
        end_user_id: None,
        team_id: None,
        org_id: None,
        project_id: None,
        kwargs: None,
        response_names: &[],
        realtime_names: &[],
        model_access_groups: &[],
        turn_environment: None,
    }
}

fn log_row(text: &'static str) -> ScriptedPayload {
    ScriptedPayload {
        script: PayloadScript::Bytes(text),
    }
}

#[tokio::test]
async fn disable_spend_updates_answers_true_without_writing() {
    let harness = Harness::new(ScriptedClaim::answering(InsertOutcome::Inserted));
    let settings = SpendWriterSettings {
        disable_spend_updates: true,
        disable_spend_logs: Some(false),
        max_budget: Decimal::ZERO,
        proxy_budget_name: "litellm-proxy-budget".to_owned(),
    };
    let request = sample_request();
    let payload = ScriptedPayload {
        script: PayloadScript::Failure,
    };
    assert!(
        harness
            .writer(settings)
            .update_database(&payload, &request)
            .await
    );
    assert!(harness.admission.work().is_empty());
    assert!(harness.claim.rows().is_empty());
    assert!(harness.log_queue.dequeue(usize::MAX).is_empty());
    assert!(harness.failures.reported().is_empty());
}

#[tokio::test]
async fn hashes_only_sk_tokens() {
    let harness = Harness::new(ScriptedClaim::answering(InsertOutcome::Inserted));
    let payload = log_row(PAYLOAD);
    let mut first = sample_request();
    first.token = Some("sk-secret");
    assert!(
        harness
            .writer(SpendWriterSettings::default())
            .update_database(&payload, &first)
            .await
    );
    let mut stored = sample_request();
    stored.token = Some("hash-1");
    assert!(
        harness
            .writer(SpendWriterSettings::default())
            .update_database(&payload, &stored)
            .await
    );
    let work = harness.admission.work();
    assert_eq!(
        work[0].hashed_token.as_deref(),
        Some(hash_token("sk-secret").as_str())
    );
    assert_eq!(work[1].hashed_token.as_deref(), Some("hash-1"));
}

#[tokio::test]
async fn a_claimed_batch_answers_false_and_stops_every_later_step() {
    let claim = ScriptedClaim::answering(InsertOutcome::Duplicate).with_existing(BatchChargeRow {
        call_type: Some("aretrieve_batch".to_owned()),
        status: Some("success".to_owned()),
        spend: Decimal::new(25, 1),
    });
    let harness = Harness::new(claim);
    let names = vec!["lookup".to_owned()];
    let mut request = sample_request();
    request.response_names = &names;
    let kwargs = JsonObject::new();
    request.kwargs = Some(&kwargs);
    assert!(
        !harness
            .writer(SpendWriterSettings::default())
            .update_database(&log_row(BATCH_PAYLOAD), &request)
            .await
    );
    assert!(harness.admission.work().is_empty());
    assert!(harness.tool_usage.is_empty());
    assert!(harness.discovery.flush().is_empty());
    assert!(harness.log_queue.dequeue(usize::MAX).is_empty());
    assert!(harness.failures.reported().is_empty());
}

#[tokio::test]
async fn a_failed_claim_queues_the_row_and_charges_on() {
    let harness = Harness::new(ScriptedClaim::answering(InsertOutcome::Failed));
    let names = vec!["lookup".to_owned()];
    let mut request = sample_request();
    request.response_names = &names;
    let kwargs = JsonObject::new();
    request.kwargs = Some(&kwargs);
    assert!(
        harness
            .writer(SpendWriterSettings::default())
            .update_database(&log_row(BATCH_PAYLOAD), &request)
            .await
    );
    let rows = harness.log_queue.dequeue(usize::MAX);
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].get("spend"),
        Some(&SpendLogValue::Decimal(Decimal::new(5, 1)))
    );
    assert_eq!(harness.admission.work().len(), 1);
    assert_eq!(harness.tool_usage.len(), 1);
    let tools: Vec<Option<String>> = harness
        .discovery
        .flush()
        .into_iter()
        .map(|item| item.tool_name)
        .collect();
    assert_eq!(tools, vec![Some("lookup".to_owned())]);
}

async fn log_dependent_counts(disable_spend_logs: Option<bool>) -> (usize, usize, usize) {
    let harness = Harness::new(ScriptedClaim::answering(InsertOutcome::Inserted));
    let names = vec!["lookup".to_owned()];
    let mut request = sample_request();
    request.response_names = &names;
    let kwargs = JsonObject::new();
    request.kwargs = Some(&kwargs);
    assert!(
        harness
            .writer(claim_settings(disable_spend_logs))
            .update_database(&log_row(PAYLOAD), &request)
            .await
    );
    (
        harness.tool_usage.len(),
        harness.discovery.flush().len(),
        harness.admission.work().len(),
    )
}

#[tokio::test]
async fn writes_log_dependent_usage_only_when_logs_are_exactly_enabled() {
    assert_eq!(log_dependent_counts(Some(false)).await, (1, 1, 1));
    assert_eq!(log_dependent_counts(Some(true)).await, (0, 1, 1));
    assert_eq!(log_dependent_counts(None).await, (0, 1, 1));
}

fn claim_settings(disable_spend_logs: Option<bool>) -> SpendWriterSettings {
    SpendWriterSettings {
        disable_spend_updates: false,
        disable_spend_logs,
        max_budget: Decimal::ZERO,
        proxy_budget_name: "litellm-proxy-budget".to_owned(),
    }
}

async fn log_rows_written(disable_spend_logs: Option<bool>) -> usize {
    let harness = Harness::new(ScriptedClaim::answering(InsertOutcome::Inserted));
    assert!(
        harness
            .writer(claim_settings(disable_spend_logs))
            .update_database(&log_row(PAYLOAD), &sample_request())
            .await
    );
    harness.log_queue.dequeue(usize::MAX).len()
}

async fn batch_claim_fields(disable_spend_logs: Option<bool>) -> Vec<String> {
    let harness = Harness::new(ScriptedClaim::answering(InsertOutcome::Inserted));
    assert!(
        harness
            .writer(claim_settings(disable_spend_logs))
            .update_database(&log_row(BATCH_PAYLOAD), &sample_request())
            .await
    );
    harness
        .claim
        .rows()
        .first()
        .expect("the batch claim wrote a row")
        .keys()
        .cloned()
        .collect()
}

#[tokio::test]
async fn a_batch_claim_with_logs_disabled_writes_only_the_claim_fields() {
    assert_eq!(
        batch_claim_fields(Some(true)).await,
        vec!["call_type", "request_id", "spend", "startTime", "status"]
    );
}

#[tokio::test]
async fn a_null_disable_spend_logs_keeps_no_log_row_and_reduces_the_claim() {
    assert_eq!(log_rows_written(Some(false)).await, 1);
    assert_eq!(log_rows_written(Some(true)).await, 0);
    assert_eq!(log_rows_written(None).await, 0);
    assert_eq!(
        batch_claim_fields(None).await,
        vec!["call_type", "request_id", "spend", "startTime", "status"]
    );
}

#[tokio::test]
async fn overwrites_spend_and_only_nonempty_ids() {
    let harness = Harness::new(ScriptedClaim::answering(InsertOutcome::Inserted));
    let mut overridden = sample_request();
    overridden.response_cost = None;
    overridden.org_id = Some("org-9");
    overridden.team_id = Some("team-9");
    assert!(
        harness
            .writer(SpendWriterSettings::default())
            .update_database(&log_row(PAYLOAD), &overridden)
            .await
    );
    let mut empty = sample_request();
    empty.org_id = Some("");
    empty.team_id = Some("");
    assert!(
        harness
            .writer(SpendWriterSettings::default())
            .update_database(&log_row(PAYLOAD), &empty)
            .await
    );
    let work = harness.admission.work();
    let first = &work[0].payload;
    assert_eq!(
        first.get("spend"),
        Some(&SpendLogValue::Decimal(Decimal::ZERO))
    );
    assert_eq!(
        first.get("organization_id"),
        Some(&SpendLogValue::Text("org-9".to_owned()))
    );
    assert_eq!(
        first.get("team_id"),
        Some(&SpendLogValue::Text("team-9".to_owned()))
    );
    assert_eq!(
        first.get("startTime"),
        Some(&SpendLogValue::Text("2025-01-02T03:04:05Z".to_owned()))
    );
    let second = &work[1].payload;
    assert!(!second.contains_key("organization_id"));
    assert!(!second.contains_key("team_id"));
}

#[tokio::test]
async fn a_failed_payload_build_is_reported_and_acknowledged() {
    let harness = Harness::new(ScriptedClaim::answering(InsertOutcome::Inserted));
    let payload = ScriptedPayload {
        script: PayloadScript::Failure,
    };
    assert!(
        harness
            .writer(SpendWriterSettings::default())
            .update_database(&payload, &sample_request())
            .await
    );
    assert_eq!(
        harness.failures.reported(),
        vec![RecordedFailure {
            operation: SpendWriterOperation::UpdateDatabase,
            token: Some("sk-secret".to_owned()),
        }]
    );
    assert!(harness.admission.work().is_empty());
}

#[tokio::test]
async fn a_closed_admission_still_acknowledges_what_was_written() {
    let harness = Harness::with_admission(
        ScriptedClaim::answering(InsertOutcome::Inserted),
        RecordingAdmission::closed(),
    );
    let names = vec!["lookup".to_owned()];
    let mut request = sample_request();
    request.response_names = &names;
    let kwargs = JsonObject::new();
    request.kwargs = Some(&kwargs);
    assert!(
        harness
            .writer(SpendWriterSettings::default())
            .update_database(&log_row(PAYLOAD), &request)
            .await
    );
    assert_eq!(
        harness.failures.reported(),
        vec![RecordedFailure {
            operation: SpendWriterOperation::UpdateDatabase,
            token: Some("sk-secret".to_owned()),
        }]
    );
    assert_eq!(harness.log_queue.dequeue(usize::MAX).len(), 1);
    assert_eq!(harness.tool_usage.len(), 1);
    assert!(harness.discovery.flush().is_empty());
}
