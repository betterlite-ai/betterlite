use std::sync::Mutex;

use betterlite_config::collector::TracebackPolicy;
use betterlite_db::redis_spend::log_row::{SpendLogRow, SpendLogValue};
use betterlite_db::spend_log_storage::batch_claim::{
    BatchChargeRow, BatchClaimTable, SpendLogAttachment,
};
use betterlite_db::spend_log_storage::rows::SpendLogQueue;
use betterlite_db::spend_log_storage::{SpendLogFuture, SpendLogWriteError, SpendLogsWriter};
use betterlite_db::spend_writer::accounting::{
    AdmissionFuture, SpendAccounting, SpendAccountingAdmission,
};
use betterlite_db::spend_writer::settings::SpendWriterSettings;
use betterlite_db::spend_writer::{
    DbSpendUpdateWriter, NoSpendWriterFailureSink, SpendWriterQueues,
};
use betterlite_logging::end_user::EndUserCostTrackingSettings;
use betterlite_logging::payload::deps::SystemPayloadClock;
use betterlite_logging::spend_bodies::SpendStoragePolicy;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_logging::spend_sanitize::SpendSanitizePolicy;
use betterlite_models::model_registration::JsonObject;
use chrono::DateTime;
use rust_decimal::Decimal;
use tokio::sync::watch;

use super::super::write::FailureSpendWrite as _;
use super::super::write::{DatabaseFailureSpendWrite, FailureSpend};
use super::support::object;
use crate::spend::payload::SpendPayloadSettings;

#[derive(Debug, thiserror::Error)]
#[error("a failure spend row is never a batch cost row")]
struct NotABatchRow;

impl SpendLogWriteError for NotABatchRow {
    fn is_transient(&self) -> bool {
        false
    }

    fn is_data_rejection(&self) -> bool {
        false
    }

    fn is_service_unavailable(&self) -> bool {
        false
    }

    fn is_deadlock(&self) -> bool {
        false
    }

    fn is_database(&self) -> bool {
        false
    }
}

impl SpendLogsWriter for NotABatchRow {
    type Error = Self;

    fn insert_rows<'call>(
        &'call self,
        _rows: &'call [SpendLogRow],
    ) -> SpendLogFuture<'call, Result<usize, Self::Error>> {
        Box::pin(async { panic!("a failure spend row never reaches the batch cost claim") })
    }
}

impl BatchClaimTable for NotABatchRow {
    fn find_batch_row<'call>(
        &'call self,
        _request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<Option<BatchChargeRow>, Self::Error>> {
        Box::pin(async { panic!("a failure spend row never reaches the batch cost claim") })
    }

    fn take_over<'call>(
        &'call self,
        _data: &'call SpendLogRow,
        _request_id: &'call str,
    ) -> SpendLogFuture<'call, Result<usize, Self::Error>> {
        Box::pin(async { panic!("a failure spend row never reaches the batch cost claim") })
    }
}

#[derive(Default)]
struct RecordingAdmission {
    work: Mutex<Vec<SpendAccounting>>,
}

impl RecordingAdmission {
    fn work(&self) -> Vec<SpendAccounting> {
        self.work.lock().expect("the admission log").clone()
    }
}

impl SpendAccountingAdmission for RecordingAdmission {
    fn admit(&self, work: SpendAccounting) -> AdmissionFuture<'_> {
        Box::pin(async move {
            self.work.lock().expect("the admission log").push(work);
            Ok(())
        })
    }
}

fn payload_settings() -> SpendPayloadSettings {
    SpendPayloadSettings {
        storage: SpendStoragePolicy {
            sanitize: SpendSanitizePolicy {
                store_bodies: false,
                max_string_length: 2048,
            },
            turn_off_message_logging: false,
        },
        missing_session_id: None,
        end_user: EndUserCostTrackingSettings::default(),
    }
}

fn timestamp(text: &str) -> SpendTimestamp {
    SpendTimestamp::Aware(DateTime::parse_from_rfc3339(text).expect("the fixture is rfc3339"))
}

fn failure_writer<'work>(
    table: &'work NotABatchRow,
    queue: &'work SpendLogQueue,
    admission: &'work RecordingAdmission,
) -> DbSpendUpdateWriter<'work, NotABatchRow> {
    DbSpendUpdateWriter {
        settings: SpendWriterSettings::default(),
        queues: SpendWriterQueues::default(),
        attachment: Some(SpendLogAttachment { table, queue }),
        failures: &NoSpendWriterFailureSink,
        admission,
    }
}

fn failure_spend(request_data: &JsonObject) -> FailureSpend<'_> {
    FailureSpend {
        request_data,
        response_result: "the provider is down",
        response_cost: Decimal::from(1_i32),
        token: Some("hashed-key"),
        user_id: Some("user-1"),
        end_user_id: None,
        team_id: Some("team-1"),
        org_id: Some("org-1"),
        project_id: Some("project-1"),
        start_time: timestamp("2024-01-01T00:00:00Z"),
        end_time: timestamp("2024-01-01T00:00:01Z"),
    }
}

fn assert_failure_attribution(queue: &SpendLogQueue, admission: &RecordingAdmission) {
    let rows = queue.dequeue(1);
    assert_eq!(rows.len(), 1, "the failure log waits for the flush");
    assert_eq!(
        rows[0].get("organization_id"),
        Some(&SpendLogValue::Text("org-1".to_owned()))
    );
    assert_eq!(
        rows[0].get("team_id"),
        Some(&SpendLogValue::Text("team-1".to_owned()))
    );
    let charged_work = admission.work();
    assert_eq!(
        charged_work.len(),
        1,
        "the charge reaches the accounting worker"
    );
    assert_eq!(charged_work[0].project_id.as_deref(), Some("project-1"));
    assert_eq!(charged_work[0].org_id.as_deref(), Some("org-1"));
}

#[tokio::test]
async fn a_failure_row_joins_the_log_queue_and_keeps_its_project_and_org() {
    let table = NotABatchRow;
    let queue = SpendLogQueue::new(usize::MAX);
    let admission = RecordingAdmission::default();
    let writer = failure_writer(&table, &queue, &admission);
    let (_sender, tracebacks) = watch::channel(TracebackPolicy::default());
    let request_data = object(
        r#"{"model": "gpt-4o",
            "litellm_params": {"metadata": {"status": "failure"}}}"#,
    );
    let spend = failure_spend(&request_data);
    assert!(
        DatabaseFailureSpendWrite {
            writer: &writer,
            settings: payload_settings(),
            cache: None,
            clock: &SystemPayloadClock,
            session_uuid: "00000000-0000-0000-0000-000000000001",
            tracebacks: &tracebacks,
        }
        .record(&spend)
        .await,
        "the failure row owns its charge"
    );
    assert_failure_attribution(&queue, &admission);
}
