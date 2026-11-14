#![cfg(test)]

pub(crate) mod support;

use std::time::Duration;

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::decisions::Json;
use rstest::rstest;
use tokio::task::yield_now;
use tokio::time::{advance, timeout};
use tokio_util::task::TaskTracker;

use crate::alert_dispatch::AlertLevel;
use crate::alert_settings::records::HangingRequestData;

use super::alert::send_hanging_request_alert;
use super::interval::poll_interval;

#[rstest]
#[case::default_threshold(300.0, 510)]
#[case::fractional_threshold(300.9, 511)]
#[case::zero_threshold(0.0, 60)]
#[case::negative_threshold(-100.0, 0)]
#[case::infinite_threshold(f64::INFINITY, 0)]
fn init_caches_with_the_alerting_ttl(#[case] threshold: f64, #[case] seconds: u64) {
    let fixture = support::fixture_with(
        support::alert_settings(threshold),
        support::FixedResolver::new(None),
    );
    assert_eq!(fixture.factory.ttls(), vec![Duration::from_secs(seconds)]);
}

#[test]
fn init_records_default_to_not_alerted_without_optional_fields() {
    let record = HangingRequestData::new("call-1".to_owned(), "gpt-4".to_owned(), support::NOW);
    assert_eq!(record.request_id, "call-1");
    assert_eq!(record.model, "gpt-4");
    assert_eq!(record.api_base, None);
    assert_eq!(record.key_alias, None);
    assert_eq!(record.team_alias, None);
    assert_eq!(record.alerting_metadata, None);
    assert_eq!(record.created_at.to_bits(), support::NOW.to_bits());
    assert!(!record.alerted);
}

#[tokio::test]
async fn add_stores_the_request_and_its_metadata() {
    let fixture = support::fixture();
    fixture.set_now(1_700_000_123.0);
    let params = support::json(
        r#"{"metadata": {"user_api_key_alias": "key-1", "user_api_key_team_alias": "team-1"}}"#,
    );
    let deployment =
        support::json(r#"{"litellm_params": {"api_base": "https://deployment.example"}}"#);
    let data = support::object(&[
        ("model", support::json("\"gpt-4\"")),
        ("litellm_call_id", support::json("\"call-1\"")),
        ("litellm_params", params),
        ("deployment", deployment),
    ]);
    fixture
        .check
        .add_hanging_request(Some(&data))
        .await
        .expect("the request is added");
    let stored = fixture
        .cache
        .stored("call-1")
        .expect("the request is cached");
    assert_eq!(stored.request_id, "call-1");
    assert_eq!(stored.model, "gpt-4");
    assert_eq!(stored.created_at.to_bits(), 1_700_000_123.0_f64.to_bits());
    assert_eq!(stored.key_alias.as_deref(), Some("key-1"));
    assert_eq!(stored.team_alias.as_deref(), Some("team-1"));
    assert_eq!(stored.api_base.as_deref(), Some("https://api.openai.com"));
    assert_eq!(stored.alerting_metadata, None);
    assert!(!stored.alerted);
    assert_eq!(
        fixture.cache.stored_ttl("call-1"),
        Some(Duration::from_secs(510))
    );
    assert_eq!(fixture.resolver.calls().len(), 1);
}

#[tokio::test]
async fn add_ignores_absent_request_data() {
    let fixture = support::fixture();
    fixture
        .check
        .add_hanging_request(None)
        .await
        .expect("absent data is not an error");
    assert_eq!(fixture.cache.entry_count(), 0);
    assert!(fixture.resolver.calls().is_empty());
}

#[tokio::test]
async fn add_records_no_api_base_without_a_deployment() {
    let fixture = support::fixture();
    let data = support::object(&[
        ("model", support::json("\"gpt-4\"")),
        ("litellm_call_id", support::json("\"call-1\"")),
    ]);
    fixture
        .check
        .add_hanging_request(Some(&data))
        .await
        .expect("the request is added");
    let stored = fixture
        .cache
        .stored("call-1")
        .expect("the request is cached");
    assert_eq!(stored.api_base, None);
    assert!(fixture.resolver.calls().is_empty());
}

#[tokio::test]
async fn add_rejects_a_field_of_the_wrong_type() {
    let fixture = support::fixture();
    let data = support::object(&[
        ("model", support::json("5")),
        ("litellm_call_id", support::json("\"call-1\"")),
    ]);
    let error = fixture
        .check
        .add_hanging_request(Some(&data))
        .await
        .expect_err("a number is not a model");
    assert_eq!(error.to_string(), "the model field must be a string");
    assert_eq!(fixture.cache.entry_count(), 0);
    assert!(fixture.resolver.calls().is_empty());
}

#[tokio::test]
async fn add_lets_the_callers_key_alias_win_over_the_internal_one() {
    let fixture = support::fixture();
    let params = support::json(
        r#"{
            "metadata": {"user_api_key_alias": "caller", "user_api_key_team_alias": "caller-team"},
            "litellm_metadata": {"user_api_key_alias": "internal", "user_api_key_team_alias": "internal-team"}
        }"#,
    );
    let data = support::object(&[
        ("model", support::json("\"gpt-4\"")),
        ("litellm_call_id", support::json("\"call-1\"")),
        ("litellm_params", params),
    ]);
    fixture
        .check
        .add_hanging_request(Some(&data))
        .await
        .expect("the request is added");
    let stored = fixture
        .cache
        .stored("call-1")
        .expect("the request is cached");
    assert_eq!(stored.key_alias.as_deref(), Some("caller"));
    assert_eq!(stored.team_alias.as_deref(), Some("caller-team"));
}

#[tokio::test]
async fn add_replaces_an_existing_request_with_the_same_id() {
    let fixture = support::fixture();
    let first = support::object(&[
        ("model", support::json("\"gpt-4\"")),
        ("litellm_call_id", support::json("\"call-1\"")),
    ]);
    let second = support::object(&[
        ("model", support::json("\"gpt-4o\"")),
        ("litellm_call_id", support::json("\"call-1\"")),
    ]);
    fixture
        .check
        .add_hanging_request(Some(&first))
        .await
        .expect("the first request is added");
    fixture
        .check
        .add_hanging_request(Some(&second))
        .await
        .expect("the second request is added");
    assert_eq!(fixture.cache.entry_count(), 1);
    assert_eq!(
        fixture
            .cache
            .stored("call-1")
            .expect("the request is cached")
            .model,
        "gpt-4o"
    );
}

#[tokio::test]
async fn add_passes_the_deployment_params_to_the_api_base_resolver() {
    let fixture = support::fixture();
    let deployment_params =
        support::object(&[("api_base", support::json("\"https://deployment.example\""))]);
    let deployment =
        support::object(&[("litellm_params", Json::Object(deployment_params.clone()))]);
    let data = support::object(&[
        ("model", support::json("\"gpt-4\"")),
        ("litellm_call_id", support::json("\"call-1\"")),
        ("deployment", Json::Object(deployment)),
    ]);
    fixture
        .check
        .add_hanging_request(Some(&data))
        .await
        .expect("the request is added");
    assert_eq!(
        fixture.resolver.calls(),
        vec![("gpt-4".to_owned(), deployment_params)]
    );
}

#[tokio::test]
async fn add_treats_a_null_field_as_absent() {
    let fixture = support::fixture();
    let data = support::object(&[
        ("model", Json::Null),
        ("litellm_call_id", support::json("\"call-9\"")),
    ]);
    fixture
        .check
        .add_hanging_request(Some(&data))
        .await
        .expect("a null model is the default");
    let stored = fixture
        .cache
        .stored("call-9")
        .expect("the request is cached");
    assert_eq!(stored.model, "");
    assert_eq!(stored.key_alias, None);
}

#[tokio::test]
async fn detect_alerts_an_old_request_once() {
    let fixture = support::fixture();
    fixture.cache.seed(support::hanging_record(
        "call-1",
        "gpt-4",
        300.0_f64.mul_add(-1.0, support::NOW),
    ));
    fixture.check.send_hanging_alerts().await;
    let alerts = fixture.sink.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].alert_type, AlertType::LlmRequestsHanging);
    assert_eq!(alerts[0].level, AlertLevel::Medium);
    assert!(
        alerts[0]
            .message
            .starts_with("`Requests are hanging - 300.0s+ request time`")
    );
    assert!(
        fixture
            .cache
            .stored("call-1")
            .expect("the request stays cached")
            .alerted
    );
    fixture.check.send_hanging_alerts().await;
    assert!(fixture.sink.take().is_empty());
}

#[rstest]
#[case::just_below(299.5)]
#[case::just_added(0.0)]
#[tokio::test]
async fn detect_skips_a_request_below_the_threshold(#[case] age: f64) {
    let fixture = support::fixture();
    fixture.cache.seed(support::hanging_record(
        "call-1",
        "gpt-4",
        age.mul_add(-1.0, support::NOW),
    ));
    fixture.check.send_hanging_alerts().await;
    assert!(fixture.sink.take().is_empty());
    assert!(!fixture.cache.stored("call-1").expect("cached").alerted);
    assert_eq!(fixture.cache.entry_count(), 1);
}

#[rstest]
#[case::completed("true")]
#[case::failed("\"fail\"")]
#[case::status_object("{\"status\": \"success\"}")]
#[tokio::test]
async fn detect_removes_a_completed_request(#[case] status: &str) {
    let fixture = support::fixture();
    fixture.cache.seed(support::hanging_record(
        "call-1",
        "gpt-4",
        300.0_f64.mul_add(-1.0, support::NOW),
    ));
    fixture
        .status
        .set("request_status:call-1", support::json(status));
    fixture.check.send_hanging_alerts().await;
    assert!(fixture.sink.take().is_empty());
    assert_eq!(fixture.cache.entry_count(), 0);
    assert_eq!(
        fixture.status.reads(),
        vec!["request_status:call-1".to_owned()]
    );
}

#[tokio::test]
async fn detect_skips_a_request_that_left_the_cache() {
    let fixture = support::fixture();
    fixture.cache.vanish("call-1");
    fixture.check.send_hanging_alerts().await;
    assert!(fixture.sink.take().is_empty());
    assert!(fixture.status.reads().is_empty());
}

#[tokio::test]
async fn detect_treats_a_null_status_as_in_flight() {
    let fixture = support::fixture();
    fixture.cache.seed(support::hanging_record(
        "call-1",
        "gpt-4",
        300.0_f64.mul_add(-1.0, support::NOW),
    ));
    fixture.status.set("request_status:call-1", Json::Null);
    fixture.check.send_hanging_alerts().await;
    assert_eq!(fixture.sink.take().len(), 1);
    assert_eq!(fixture.cache.entry_count(), 1);
}

#[tokio::test]
async fn detect_checks_only_the_oldest_twenty_requests() {
    let fixture = support::fixture();
    (0..21).for_each(|index| {
        fixture.cache.seed(support::hanging_record(
            &format!("call-{index}"),
            "gpt-4",
            400.0_f64.mul_add(-1.0, support::NOW),
        ));
    });
    fixture.check.send_hanging_alerts().await;
    assert_eq!(fixture.sink.take().len(), 20);
    assert!(fixture.cache.stored("call-19").expect("cached").alerted);
    assert!(!fixture.cache.stored("call-20").expect("cached").alerted);
    assert_eq!(fixture.cache.entry_count(), 21);
}

#[tokio::test]
async fn detect_skips_an_alerted_request() {
    let fixture = support::fixture();
    let mut record =
        support::hanging_record("call-1", "gpt-4", 300.0_f64.mul_add(-1.0, support::NOW));
    record.alerted = true;
    fixture.cache.seed(record);
    fixture.check.send_hanging_alerts().await;
    assert!(fixture.sink.take().is_empty());
    assert_eq!(fixture.cache.entry_count(), 1);
}

#[tokio::test]
async fn send_formats_every_field_of_the_hanging_alert() {
    let fixture = support::fixture();
    let mut record = support::hanging_record("call-1", "gpt-4", support::NOW);
    record.api_base = Some("https://api.openai.com".to_owned());
    record.key_alias = Some("key-1".to_owned());
    record.team_alias = Some("team-1".to_owned());
    send_hanging_request_alert(
        fixture.sink.as_ref(),
        &support::alert_settings(300.0),
        &record,
    )
    .await;
    let alerts = fixture.sink.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(
        alerts[0].message,
        "`Requests are hanging - 300.0s+ request time`\nRequest Model: `gpt-4`\nAPI Base: `https://api.openai.com`\nKey Alias: `key-1`\nTeam Alias: `team-1`"
    );
    assert_eq!(alerts[0].level, AlertLevel::Medium);
    assert_eq!(alerts[0].alert_type, AlertType::LlmRequestsHanging);
    assert_eq!(alerts[0].request_model.as_deref(), Some("gpt-4"));
    assert_eq!(
        alerts[0].api_base.as_deref(),
        Some("https://api.openai.com")
    );
    assert!(alerts[0].alerting_metadata.is_empty());
    assert!(alerts[0].kwargs.is_empty());
}

#[tokio::test]
async fn send_uses_empty_strings_for_missing_values() {
    let fixture = support::fixture();
    let record = support::hanging_record("call-1", "gpt-4", support::NOW);
    send_hanging_request_alert(
        fixture.sink.as_ref(),
        &support::alert_settings(300.0),
        &record,
    )
    .await;
    let alerts = fixture.sink.take();
    assert_eq!(
        alerts[0].message,
        "`Requests are hanging - 300.0s+ request time`\nRequest Model: `gpt-4`\nAPI Base: ``\nKey Alias: ``\nTeam Alias: ``"
    );
    assert_eq!(alerts[0].api_base, None);
}

#[tokio::test]
async fn send_passes_the_alerting_metadata() {
    let fixture = support::fixture();
    let expected = support::object(&[("team", support::json("\"team-1\""))]);
    let mut record = support::hanging_record("call-1", "gpt-4", support::NOW);
    record.alerting_metadata = Some(expected.clone());
    send_hanging_request_alert(
        fixture.sink.as_ref(),
        &support::alert_settings(300.0),
        &record,
    )
    .await;
    let alerts = fixture.sink.take();
    assert_eq!(alerts[0].alerting_metadata, expected);
}

#[rstest]
#[case::whole(300.0, "300.0")]
#[case::fractional(0.5, "0.5")]
#[case::another_whole(60.0, "60.0")]
#[tokio::test]
async fn send_renders_the_threshold_as_a_python_float(#[case] threshold: f64, #[case] text: &str) {
    let fixture = support::fixture();
    let record = support::hanging_record("call-1", "gpt-4", support::NOW);
    send_hanging_request_alert(
        fixture.sink.as_ref(),
        &support::alert_settings(threshold),
        &record,
    )
    .await;
    let alerts = fixture.sink.take();
    assert!(
        alerts[0]
            .message
            .contains(&format!("`Requests are hanging - {text}s+ request time`"))
    );
}

#[rstest]
#[case::five_minutes(300.0, 150)]
#[case::two_seconds(2.0, 1)]
#[case::below_the_floor(1.0, 1)]
#[case::zero_threshold(0.0, 1)]
#[case::negative_threshold(-10.0, 1)]
fn loop_polls_every_half_the_threshold(#[case] threshold: f64, #[case] seconds: u64) {
    assert_eq!(poll_interval(threshold), Duration::from_secs(seconds));
}

#[tokio::test(start_paused = true)]
async fn loop_runs_checks_until_the_tracker_closes() {
    let fixture = support::fixture_with(
        support::alert_settings(8.0),
        support::FixedResolver::new(None),
    );
    let tasks = TaskTracker::new();
    fixture.check.ensure_started(&tasks);
    yield_now().await;
    assert_eq!(fixture.cache.oldest_calls(), 1);
    advance(Duration::from_secs(3)).await;
    yield_now().await;
    assert_eq!(fixture.cache.oldest_calls(), 1);
    advance(Duration::from_secs(1)).await;
    yield_now().await;
    assert_eq!(fixture.cache.oldest_calls(), 2);
    tasks.close();
    advance(Duration::from_secs(4)).await;
    timeout(Duration::from_secs(5), tasks.wait())
        .await
        .expect("the check loop stops after close");
}

#[tokio::test(start_paused = true)]
async fn loop_uses_the_live_threshold_for_the_next_interval() {
    let fixture = support::fixture_with(
        support::alert_settings(8.0),
        support::FixedResolver::new(None),
    );
    let tasks = TaskTracker::new();
    fixture.check.ensure_started(&tasks);
    yield_now().await;
    assert_eq!(fixture.cache.oldest_calls(), 1);
    fixture.settings.set_threshold(16.0);
    advance(Duration::from_secs(4)).await;
    yield_now().await;
    assert_eq!(fixture.cache.oldest_calls(), 2);
    advance(Duration::from_secs(4)).await;
    yield_now().await;
    assert_eq!(fixture.cache.oldest_calls(), 2);
    advance(Duration::from_secs(4)).await;
    yield_now().await;
    assert_eq!(fixture.cache.oldest_calls(), 3);
    tasks.close();
    advance(Duration::from_secs(8)).await;
    timeout(Duration::from_secs(5), tasks.wait())
        .await
        .expect("the check loop stops after close");
}

#[tokio::test(start_paused = true)]
async fn loop_starts_only_one_task() {
    let fixture = support::fixture_with(
        support::alert_settings(8.0),
        support::FixedResolver::new(None),
    );
    let tasks = TaskTracker::new();
    fixture.check.ensure_started(&tasks);
    fixture.check.ensure_started(&tasks);
    yield_now().await;
    assert_eq!(fixture.cache.oldest_calls(), 1);
    tasks.close();
    advance(Duration::from_secs(4)).await;
    timeout(Duration::from_secs(5), tasks.wait())
        .await
        .expect("the check loop stops after close");
}
