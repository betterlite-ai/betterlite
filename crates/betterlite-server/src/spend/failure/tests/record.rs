use betterlite_auth::reservation_settlement::handle::ReservationHandle;
use betterlite_db::database_failures::deadline::DbLookupDeadlineExceeded;
use betterlite_error::normalize::Normalizer;
use betterlite_logging::logging_metadata::error_info::ExceptionLogging;
use betterlite_logging::logging_metadata::exception::ExceptionFacts;
use betterlite_logging::spend_bodies::session::SpendTimestamp;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use betterlite_redis::request_batch::lock;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use super::super::enrich::FailureIdentityLookup;
use super::super::fields::CallLog;
use super::super::metadata::as_object;
use super::super::record::{
    FailurePorts, FailureRecord, FailureRecordError, FailureRecording, FailureReservation,
    FailureSettings, record_failure,
};
use super::support::{
    FixedClock, Harness, RecordingWriter, Settlement, SettlementOutcome, SpyLookup, Written,
    nested, object, reservation, settings,
};

struct Scenario {
    harness: Harness,
    writer: RecordingWriter,
    clock: FixedClock,
    settlement: Settlement,
    handle: ReservationHandle,
    lookup: SpyLookup,
    failure_settings: FailureSettings,
    call_log: Option<CallLog>,
    deadline: Option<DbLookupDeadlineExceeded>,
    resolve: bool,
    traceback: Option<&'static str>,
    response_result: &'static str,
}

impl Scenario {
    fn new() -> Self {
        Self {
            harness: Harness::new(),
            writer: RecordingWriter::new(true),
            clock: FixedClock(now()),
            settlement: Settlement::new(SettlementOutcome::BothSucceed),
            handle: reservation(),
            lookup: SpyLookup::default(),
            failure_settings: settings(),
            call_log: None,
            deadline: None,
            resolve: true,
            traceback: Some("the stack"),
            response_result: "the provider is down",
        }
    }

    fn with_request(mut self, request: &str) -> Self {
        self.harness.request_data = object(request);
        self
    }

    fn with_route(mut self, route: Option<&str>) -> Self {
        self.harness.principal.request_route = route.map(str::to_owned);
        self
    }

    fn with_charge(mut self, charged: bool) -> Self {
        self.writer = RecordingWriter::new(charged);
        self
    }

    fn settled(&self) -> bool {
        lock(&self.handle.0).finalized
    }

    async fn run(&mut self) -> Result<FailureRecording, FailureRecordError> {
        let identity: Option<&dyn FailureIdentityLookup> = Some(&self.lookup);
        let record = FailureRecord {
            request_data: &mut self.harness.request_data,
            principal: &self.harness.principal,
            logging: logging(&self.harness.facts, &self.harness.normalizer),
            traceback: self.traceback,
            model_not_found: None,
            response_result: self.response_result,
            deadline: self.deadline.as_ref(),
            call_log: self.call_log.as_ref(),
            identity,
            resolve_missing_key_identity: self.resolve,
        };
        let ports = FailurePorts {
            reservation: Some(FailureReservation {
                settlement: &self.settlement,
                handle: &self.handle,
            }),
            writer: &self.writer,
            clock: &self.clock,
        };
        record_failure(record, &self.failure_settings, &ports).await
    }

    fn rows(&self) -> Vec<Written> {
        self.writer.written()
    }

    fn only_row(&self) -> Written {
        let rows = self.rows();
        assert_eq!(rows.len(), 1, "the failure wrote one row");
        rows.into_iter().next().expect("the row exists")
    }
}

fn now() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2024-05-05T05:05:05Z")
        .expect("the fixture is rfc3339")
        .with_timezone(&Utc)
}

fn logging<'facts>(
    facts: &'facts ExceptionFacts,
    normalizer: &'facts Normalizer,
) -> ExceptionLogging<'facts> {
    ExceptionLogging {
        exception: Some(facts),
        log_client_error_tracebacks: true,
        expected_client_error: false,
        redactor: None,
        normalizer,
    }
}

fn metadata(row: &Written) -> JsonObject {
    let params = as_object(&row.request_data["litellm_params"]).expect("the params are an object");
    as_object(&params["metadata"])
        .expect("the merged metadata is an object")
        .clone()
}

#[tokio::test]
async fn a_disabled_hook_still_releases_the_reservation() {
    let mut scenario = Scenario::new();
    scenario.failure_settings.disable_error_logs = true;
    let outcome = scenario.run().await.expect("recording succeeds");
    assert_eq!(outcome, FailureRecording::Skipped);
    assert_eq!(scenario.settlement.calls(), vec!["release"]);
    assert!(!scenario.settled());
    assert!(scenario.rows().is_empty());
}

#[tokio::test]
async fn an_ineligible_route_still_releases_the_reservation() {
    let mut scenario = Scenario::new().with_route(Some("/v1/unknown"));
    let outcome = scenario.run().await.expect("recording succeeds");
    assert_eq!(outcome, FailureRecording::Skipped);
    assert_eq!(scenario.settlement.calls(), vec!["release"]);
}

#[tokio::test]
async fn a_failed_release_invalidates_and_finalizes_on_success() {
    let mut scenario = Scenario::new();
    scenario.settlement = Settlement::new(SettlementOutcome::ReleaseFails);
    let outcome = scenario.run().await.expect("recording succeeds");
    assert_eq!(outcome, FailureRecording::Recorded { charged: true });
    assert_eq!(scenario.settlement.calls(), vec!["release", "invalidate"]);
    assert!(scenario.settled());
}

#[tokio::test]
async fn a_failed_invalidation_leaves_the_reservation_unfinalized() {
    let mut scenario = Scenario::new();
    scenario.settlement = Settlement::new(SettlementOutcome::BothFail);
    scenario.run().await.expect("recording succeeds");
    assert_eq!(scenario.settlement.calls(), vec!["release", "invalidate"]);
    assert!(!scenario.settled());
}

#[tokio::test]
async fn a_null_route_records_the_failure_and_its_identity() {
    let mut scenario = Scenario::new().with_request(
        r#"{"model": "gpt-4o", "metadata": {"keep": 1},
            "litellm_params": {"model": "gpt-4o", "custom_llm_provider": "openai"}}"#,
    );
    let outcome = scenario.run().await.expect("recording succeeds");
    assert_eq!(outcome, FailureRecording::Recorded { charged: true });
    let row = scenario.only_row();
    assert_eq!(row.token.as_deref(), Some("hashed-key"));
    assert_eq!(row.user_id.as_deref(), Some("user-1"));
    assert_eq!(row.end_user_id, None);
    assert_eq!(row.team_id.as_deref(), Some("team-1"));
    assert_eq!(row.org_id.as_deref(), Some("org-1"));
    assert_eq!(row.project_id.as_deref(), Some("project-1"));
    assert_eq!(row.response_result, "the provider is down");
    let merged = metadata(&row);
    assert_eq!(merged["status"], Json::Text("failure".to_owned()));
    assert_eq!(merged["keep"], Json::Number(serde_json::Number::from(1_u8)));
    assert!(merged.contains_key("error_information"));
    assert_eq!(merged["used_client_oauth_token"], Json::Null);
    assert_eq!(row.request_data["model"], Json::Text("gpt-4o".to_owned()));
    assert_eq!(
        row.request_data["custom_llm_provider"],
        Json::Text("openai".to_owned())
    );
    assert_eq!(
        *nested(&row.request_data, &["metadata", "status"]),
        Json::Text("failure".to_owned())
    );
}

#[tokio::test]
async fn llm_info_and_another_route_split_on_eligibility() {
    let mut chat = Scenario::new().with_route(Some("/v1/chat/completions"));
    assert_eq!(
        chat.run().await.expect("recording succeeds"),
        FailureRecording::Recorded { charged: true }
    );
    let mut info = Scenario::new().with_route(Some("/health"));
    assert_eq!(
        info.run().await.expect("recording succeeds"),
        FailureRecording::Recorded { charged: true }
    );
    let mut other = Scenario::new().with_route(Some("/v1/unknown"));
    assert_eq!(
        other.run().await.expect("recording succeeds"),
        FailureRecording::Skipped
    );
}

#[tokio::test]
async fn the_default_start_and_end_are_the_clock_now() {
    let mut scenario = Scenario::new();
    scenario.run().await.expect("recording succeeds");
    let row = scenario.only_row();
    let stamp = SpendTimestamp::Aware(now().fixed_offset());
    assert_eq!(row.start_time, stamp);
    assert_eq!(row.end_time, stamp);
}

#[tokio::test]
async fn the_call_log_supplies_the_start_trace_and_standard_logging_object() {
    let mut scenario = Scenario::new().with_request("{}");
    let logged_start = DateTime::parse_from_rfc3339("2024-01-01T00:00:00Z")
        .expect("the fixture is rfc3339")
        .with_timezone(&Utc);
    scenario.call_log = Some(CallLog {
        standard_logging_object: Some(Json::Object(object(r#"{"id": "call"}"#))),
        litellm_trace_id: Some("trace".to_owned()),
        start_time: Some(SpendTimestamp::Aware(logged_start.fixed_offset())),
    });
    scenario.run().await.expect("recording succeeds");
    let row = scenario.only_row();
    assert_eq!(
        row.start_time,
        SpendTimestamp::Aware(logged_start.fixed_offset())
    );
    assert_eq!(
        row.request_data["litellm_trace_id"],
        Json::Text("trace".to_owned())
    );
    assert_eq!(
        row.request_data["standard_logging_object"],
        Json::Object(object(r#"{"id": "call"}"#))
    );
}

#[tokio::test]
async fn a_deadline_skips_all_enrichment_but_keeps_the_stamp() {
    let mut scenario = Scenario::new().with_request(
        r#"{"user_api_key": "hash", "user_api_key_alias": null,
            "user_api_key_team_id": "team", "user_api_key_team_alias": null}"#,
    );
    scenario.deadline = Some(DbLookupDeadlineExceeded {
        lookup: "key".to_owned(),
        deadline_seconds: 1.5,
        cause: None,
    });
    scenario.run().await.expect("recording succeeds");
    assert_eq!(scenario.lookup.key_calls(), 0);
    assert_eq!(scenario.lookup.team_calls(), 0);
    let merged = metadata(&scenario.only_row());
    assert_eq!(merged["user_api_key_alias"], Json::Null);
    assert_eq!(merged["status"], Json::Text("failure".to_owned()));
}

#[tokio::test]
async fn a_live_failure_enriches_the_key_and_team_identity() {
    let mut scenario = Scenario::new().with_request(
        r#"{"user_api_key": "hash", "user_api_key_alias": null,
            "user_api_key_team_id": "team", "user_api_key_team_alias": null}"#,
    );
    scenario.run().await.expect("recording succeeds");
    assert_eq!(scenario.lookup.key_calls(), 1);
    assert_eq!(scenario.lookup.team_calls(), 1);
    let merged = metadata(&scenario.only_row());
    assert_eq!(
        merged["user_api_key_alias"],
        Json::Text("looked-up-alias".to_owned())
    );
    assert_eq!(
        merged["user_api_key_team_alias"],
        Json::Text("looked-up-team".to_owned())
    );
}

#[tokio::test]
async fn a_delayed_event_keeps_its_identity_and_still_fills_the_team_alias() {
    let mut scenario = Scenario::new().with_request(
        r#"{"user_api_key": "hash", "user_api_key_alias": null,
                "user_api_key_team_id": "team", "user_api_key_team_alias": null}"#,
    );
    scenario.resolve = false;
    scenario.run().await.expect("recording succeeds");
    assert_eq!(scenario.lookup.key_calls(), 0);
    assert_eq!(scenario.lookup.team_calls(), 1);
    let merged = metadata(&scenario.only_row());
    assert_eq!(merged["user_api_key_alias"], Json::Null);
    assert_eq!(
        merged["user_api_key_team_alias"],
        Json::Text("looked-up-team".to_owned())
    );
}

#[tokio::test]
async fn recovered_stream_cost_and_guardrail_charges_are_written() {
    let mut scenario = Scenario::new().with_request(
        r#"{"combined_usage_object": {}, "response_cost": 2.5,
            "litellm_metadata": {"standard_logging_guardrail_information": [{"guardrail_cost": 0.5}]}}"#,
    );
    scenario.run().await.expect("recording succeeds");
    let row = scenario.only_row();
    assert_eq!(row.response_cost, Decimal::from(3_i32));
}

#[tokio::test]
async fn a_text_response_cost_fails_before_the_write() {
    let mut scenario =
        Scenario::new().with_request(r#"{"combined_usage_object": {}, "response_cost": "two"}"#);
    let error = scenario.run().await.expect_err("the cost is rejected");
    assert!(matches!(error, FailureRecordError::Cost(_)));
    assert_eq!(scenario.settlement.calls(), vec!["release"]);
    assert!(scenario.rows().is_empty());
}

#[tokio::test]
async fn the_write_ownership_is_reported() {
    let mut scenario = Scenario::new().with_charge(false);
    let outcome = scenario.run().await.expect("recording succeeds");
    assert_eq!(outcome, FailureRecording::Recorded { charged: false });
}

#[tokio::test]
async fn an_llm_route_carries_the_oauth_provenance_from_its_bucket() {
    let mut scenario = Scenario::new()
        .with_route(Some("/v1/messages"))
        .with_request(r#"{"litellm_metadata": {"used_client_oauth_token": true}}"#);
    scenario.run().await.expect("recording succeeds");
    let merged = metadata(&scenario.only_row());
    assert_eq!(merged["used_client_oauth_token"], Json::Bool(true));
}
