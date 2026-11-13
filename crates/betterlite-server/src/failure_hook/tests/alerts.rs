use std::sync::Arc;
use std::time::Duration;

use rstest::rstest;

use super::super::alerts::{AlertError, AlertRequestData, ProxyAlerts, alerting_metadata_from};
use betterlite_openai::openai_base_config::Object as JsonMap;

use super::super::ports::{
    AlertLevel, AlertType, AlertingClient, BudgetAlertKind, Clock, FailureLogging, FailureService,
    RequestStatus,
};
use super::support::{
    AlertParts, FakeEmail, FakeLogging, FakeSentry, FakeServiceLogging, FakeSlack, FakeUsageCache,
    Journal, SteppingClock, call_info, drain, http_error, json, object, proxy_alerts, redactor,
};

#[tokio::test]
async fn update_request_status_writes_nothing_without_alerting() {
    let journal = Journal::default();
    let alerts = proxy_alerts(
        AlertParts::new(None, Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::new(FakeSlack::new(&journal, true)),
        Arc::new(FakeServiceLogging::default()),
    );
    alerts
        .update_request_status("abc", RequestStatus::Fail)
        .await;
    assert!(journal.entries().is_empty());
}

#[tokio::test]
async fn update_request_status_keeps_the_threshold_buffer_and_local_flag() {
    let journal = Journal::default();
    let alerts = proxy_alerts(
        AlertParts::new(Some(vec![AlertingClient::Slack]), Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::new(FakeSlack::new(&journal, true)),
        Arc::new(FakeServiceLogging::default()),
    );
    alerts
        .update_request_status("abc", RequestStatus::Fail)
        .await;
    assert_eq!(
        journal.entries(),
        vec!["status:request_status:abc:fail:400".to_owned()]
    );
}

#[tokio::test]
async fn failed_tracking_alert_reaches_slack_only_with_alerting() {
    let journal = Journal::default();
    let slack = Arc::new(FakeSlack::new(&journal, true));
    let quiet = proxy_alerts(
        AlertParts::new(None, Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&slack),
        Arc::new(FakeServiceLogging::default()),
    );
    quiet.failed_tracking_alert("boom", "gpt-4").await;
    assert!(slack.failed_tracking().is_empty());
    let loud = proxy_alerts(
        AlertParts::new(Some(vec![AlertingClient::Slack]), Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&slack),
        Arc::new(FakeServiceLogging::default()),
    );
    loud.failed_tracking_alert("boom", "gpt-4").await;
    assert_eq!(
        slack.failed_tracking(),
        vec![("boom".to_owned(), "gpt-4".to_owned())]
    );
}

struct BudgetCase {
    alerting: Option<Vec<AlertingClient>>,
    kind: BudgetAlertKind,
    emails: Option<Vec<String>>,
    email_wired: bool,
    slack_calls: usize,
    email_calls: usize,
}

#[rstest]
#[case::alerting_off_user_budget(BudgetCase {
    alerting: None,
    kind: BudgetAlertKind::UserBudget,
    emails: None,
    email_wired: true,
    slack_calls: 0,
    email_calls: 0,
})]
#[case::soft_budget_with_emails_but_no_logger(BudgetCase {
    alerting: None,
    kind: BudgetAlertKind::SoftBudget,
    emails: Some(vec!["a@example.com".to_owned()]),
    email_wired: false,
    slack_calls: 0,
    email_calls: 0,
})]
#[case::soft_budget_emails_bypass_the_guard(BudgetCase {
    alerting: None,
    kind: BudgetAlertKind::SoftBudget,
    emails: Some(vec!["a@example.com".to_owned()]),
    email_wired: true,
    slack_calls: 0,
    email_calls: 1,
})]
#[case::soft_budget_without_emails_stops(BudgetCase {
    alerting: None,
    kind: BudgetAlertKind::SoftBudget,
    emails: Some(Vec::new()),
    email_wired: true,
    slack_calls: 0,
    email_calls: 0,
})]
#[case::ms_teams_uses_the_slack_instance(BudgetCase {
    alerting: Some(vec![AlertingClient::MsTeams]),
    kind: BudgetAlertKind::UserBudget,
    emails: None,
    email_wired: false,
    slack_calls: 1,
    email_calls: 0,
})]
#[case::slack(BudgetCase {
    alerting: Some(vec![AlertingClient::Slack]),
    kind: BudgetAlertKind::UserBudget,
    emails: None,
    email_wired: false,
    slack_calls: 1,
    email_calls: 0,
})]
#[case::webhook_uses_the_slack_instance(BudgetCase {
    alerting: Some(vec![AlertingClient::Webhook]),
    kind: BudgetAlertKind::TeamBudget,
    emails: None,
    email_wired: false,
    slack_calls: 1,
    email_calls: 0,
})]
#[case::email_only(BudgetCase {
    alerting: Some(vec![AlertingClient::Email]),
    kind: BudgetAlertKind::OrganizationBudget,
    emails: None,
    email_wired: true,
    slack_calls: 0,
    email_calls: 1,
})]
#[case::slack_and_email(BudgetCase {
    alerting: Some(vec![AlertingClient::Slack, AlertingClient::Email]),
    kind: BudgetAlertKind::UserBudget,
    emails: None,
    email_wired: true,
    slack_calls: 1,
    email_calls: 1,
})]
#[tokio::test]
async fn budget_alerts_follow_the_alerting_flags(#[case] case: BudgetCase) {
    let journal = Journal::default();
    let slack = Arc::new(FakeSlack::new(&journal, true));
    let email = Arc::new(FakeEmail::default());
    let parts = AlertParts {
        alerting: case.alerting,
        alert_types: Vec::new(),
        proxy_base_url: None,
        alerting_threshold: Duration::from_secs(300),
        email: case.email_wired.then(|| Arc::clone(&email)),
        sentry: None,
        redactor: None,
        clock: None,
    };
    let alerts = proxy_alerts(
        parts,
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&slack),
        Arc::new(FakeServiceLogging::default()),
    );
    alerts
        .budget_alerts(case.kind, &call_info(case.emails))
        .await;
    assert_eq!(slack.budget().len(), case.slack_calls);
    assert_eq!(email.budget().len(), case.email_calls);
}

struct AlertFixtures {
    alerts: Arc<ProxyAlerts>,
    slack: Arc<FakeSlack>,
    sentry: Arc<FakeSentry>,
}

fn sentry_fixtures(journal: &Journal) -> AlertFixtures {
    let slack = Arc::new(FakeSlack::new(journal, true));
    let sentry = Arc::new(FakeSentry::default());
    let mut parts = AlertParts::new(
        Some(vec![AlertingClient::Slack, AlertingClient::Sentry]),
        vec![AlertType::LlmExceptions],
    );
    parts.proxy_base_url = Some("https://proxy.example".to_owned());
    parts.sentry = Some(Arc::clone(&sentry));
    parts.redactor = Some(redactor());
    let alerts = proxy_alerts(
        parts,
        Arc::new(FakeUsageCache::new(journal)),
        Arc::clone(&slack),
        Arc::new(FakeServiceLogging::default()),
    );
    AlertFixtures {
        alerts,
        slack,
        sentry,
    }
}

async fn send_langfuse_alert(fixtures: &AlertFixtures, journal: &Journal) {
    let logging: Arc<dyn FailureLogging> = Arc::new(FakeLogging::new(
        journal,
        false,
        Some("https://langfuse.example/trace/abc".to_owned()),
    ));
    let request = AlertRequestData {
        alerting_metadata: alerting_metadata_from(&object(&[(
            "metadata",
            json(r#"{"alerting_metadata":{"team":"t1"}}"#),
        )])),
        logging: Some(logging),
    };
    fixtures
        .alerts
        .alerting_handler(
            "boom",
            AlertLevel::High,
            AlertType::LlmExceptions,
            Some(request),
        )
        .await
        .expect("alert sent");
}

fn assert_formatted_alert(fixtures: &AlertFixtures) {
    let sent = fixtures.slack.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, "boom");
    assert_eq!(sent[0].level, AlertLevel::High);
    assert_eq!(sent[0].alert_type, AlertType::LlmExceptions);
    assert_eq!(sent[0].alerting_metadata.get("team"), Some(&json("\"t1\"")));
    assert_eq!(
        sent[0].langfuse_trace.as_deref(),
        Some("https://langfuse.example/trace/abc")
    );
    assert_eq!(
        fixtures.sentry.messages(),
        vec![
            "Level: `High`\nTimestamp: `12:34:56`\n\nMessage: boom\n\nProxy URL: `https://proxy.example`\n\n\u{1faa2} Langfuse Trace: https://langfuse.example/trace/abc"
                .to_owned()
        ]
    );
}

#[tokio::test]
async fn alerting_handler_formats_the_message_and_routes_it() {
    let journal = Journal::default();
    let fixtures = sentry_fixtures(&journal);
    send_langfuse_alert(&fixtures, &journal).await;
    assert_formatted_alert(&fixtures);
}

#[tokio::test]
async fn alerting_handler_fails_without_a_sentry_instance() {
    let journal = Journal::default();
    let alerts = proxy_alerts(
        AlertParts::new(Some(vec![AlertingClient::Sentry]), Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::new(FakeSlack::new(&journal, true)),
        Arc::new(FakeServiceLogging::default()),
    );
    let error = alerts
        .alerting_handler("boom", AlertLevel::High, AlertType::DbExceptions, None)
        .await
        .expect_err("missing sentry");
    assert_eq!(error, AlertError::MissingSentryDsn);
    assert_eq!(error.to_string(), "Missing SENTRY_DSN from environment");
    let quiet = proxy_alerts(
        AlertParts::new(None, Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::new(FakeSlack::new(&journal, true)),
        Arc::new(FakeServiceLogging::default()),
    );
    quiet
        .alerting_handler("boom", AlertLevel::High, AlertType::DbExceptions, None)
        .await
        .expect("alerting off is not a failure");
}

fn sentry_alert_with_clock(
    journal: &Journal,
    sentry: &Arc<FakeSentry>,
    clock: Arc<dyn Clock>,
) -> Arc<ProxyAlerts> {
    let mut parts = AlertParts::new(Some(vec![AlertingClient::Sentry]), Vec::new());
    parts.sentry = Some(Arc::clone(sentry));
    parts.clock = Some(clock);
    proxy_alerts(
        parts,
        Arc::new(FakeUsageCache::new(journal)),
        Arc::new(FakeSlack::new(journal, true)),
        Arc::new(FakeServiceLogging::default()),
    )
}

#[tokio::test]
async fn the_alert_timestamp_is_taken_before_the_langfuse_lookup() {
    let journal = Journal::default();
    let clock = Arc::new(SteppingClock::new(
        "2026-01-01T12:00:00+00:00",
        "2026-01-01T12:00:09+00:00",
    ));
    let advancing = Arc::clone(&clock);
    let sentry = Arc::new(FakeSentry::default());
    let shared_clock: Arc<dyn Clock> = clock;
    let alerts = sentry_alert_with_clock(&journal, &sentry, shared_clock);
    let logging: Arc<dyn FailureLogging> = Arc::new(
        FakeLogging::new(
            &journal,
            false,
            Some("https://langfuse.example/trace/abc".to_owned()),
        )
        .with_langfuse_hook(Arc::new(move || advancing.advance())),
    );
    let request = AlertRequestData {
        alerting_metadata: JsonMap::new(),
        logging: Some(logging),
    };
    alerts
        .alerting_handler(
            "boom",
            AlertLevel::High,
            AlertType::LlmExceptions,
            Some(request),
        )
        .await
        .expect("sent");
    assert_eq!(
        sentry.messages(),
        vec![
            "Level: `High`\nTimestamp: `12:00:00`\n\nMessage: boom\n\n\u{1faa2} Langfuse Trace: https://langfuse.example/trace/abc"
                .to_owned()
        ]
    );
}

struct SendCase {
    alerting: Vec<AlertingClient>,
    sends: usize,
}

#[rstest]
#[case::slack(SendCase {
    alerting: vec![AlertingClient::Slack],
    sends: 1,
})]
#[case::ms_teams(SendCase {
    alerting: vec![AlertingClient::MsTeams],
    sends: 1,
})]
#[case::webhook_skips_the_slack_send(SendCase {
    alerting: vec![AlertingClient::Webhook],
    sends: 0,
})]
#[tokio::test]
async fn alerting_handler_sends_only_to_slack_and_ms_teams(#[case] case: SendCase) {
    let journal = Journal::default();
    let slack = Arc::new(FakeSlack::new(&journal, true));
    let alerts = proxy_alerts(
        AlertParts::new(Some(case.alerting), Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&slack),
        Arc::new(FakeServiceLogging::default()),
    );
    alerts
        .alerting_handler("boom", AlertLevel::High, AlertType::LlmExceptions, None)
        .await
        .expect("sent");
    assert_eq!(slack.sent().len(), case.sends);
}

#[tokio::test]
async fn failure_handler_skips_everything_without_db_exceptions() {
    let journal = Journal::default();
    let slack = Arc::new(FakeSlack::new(&journal, true));
    let sentry = Arc::new(FakeSentry::default());
    let service = Arc::new(FakeServiceLogging::default());
    let parts = AlertParts {
        alerting: Some(vec![AlertingClient::Sentry]),
        alert_types: Vec::new(),
        proxy_base_url: None,
        alerting_threshold: Duration::from_secs(300),
        email: None,
        sentry: Some(Arc::clone(&sentry)),
        redactor: None,
        clock: None,
    };
    let alerts = proxy_alerts(
        parts,
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&slack),
        Arc::clone(&service),
    );
    alerts
        .failure_handler(
            &http_error(500, "db broke"),
            Duration::from_secs(2),
            "completion",
            "trace",
        )
        .await;
    drain(&alerts.tasks).await;
    assert!(slack.sent().is_empty());
    assert!(service.records().is_empty());
    assert!(sentry.exceptions().is_empty());
}

#[tokio::test]
async fn failure_handler_reports_the_redacted_message_and_the_service_failure() {
    let journal = Journal::default();
    let slack = Arc::new(FakeSlack::new(&journal, true));
    let sentry = Arc::new(FakeSentry::default());
    let service = Arc::new(FakeServiceLogging::default());
    let parts = AlertParts {
        alerting: Some(vec![AlertingClient::Slack]),
        alert_types: vec![AlertType::DbExceptions],
        proxy_base_url: None,
        alerting_threshold: Duration::from_secs(300),
        email: None,
        sentry: Some(Arc::clone(&sentry)),
        redactor: Some(redactor()),
        clock: None,
    };
    let alerts = proxy_alerts(
        parts,
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&slack),
        Arc::clone(&service),
    );
    let error = http_error(500, "db broke sk-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    let traceback = " ".repeat(1200);
    alerts
        .failure_handler(&error, Duration::from_secs(2), "completion", &traceback)
        .await;
    drain(&alerts.tasks).await;
    let expected = format!("db broke REDACTED{}", " ".repeat(1000));
    let records = service.records();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].service, FailureService::Db);
    assert_eq!(records[0].duration, Duration::from_secs(2));
    assert_eq!(records[0].call_type, "completion");
    assert_eq!(records[0].error, expected);
    let sent = slack.sent();
    assert_eq!(sent.len(), 1);
    assert_eq!(
        sent[0].message,
        format!("DB read/write call failed: {expected}")
    );
    assert_eq!(sentry.exceptions(), vec![error.to_string()]);
}

#[tokio::test]
async fn start_hanging_request_check_starts_only_when_configured() {
    let journal = Journal::default();
    let quiet_slack = Arc::new(FakeSlack::new(&journal, false));
    let quiet = proxy_alerts(
        AlertParts::new(Some(vec![AlertingClient::Slack]), Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&quiet_slack),
        Arc::new(FakeServiceLogging::default()),
    );
    quiet.start_hanging_request_check(Some(object(&[("model", json("\"gpt-4\""))])));
    drain(&quiet.tasks).await;
    assert!(quiet_slack.too_long().is_empty());
    let loud_slack = Arc::new(FakeSlack::new(&journal, true));
    let loud = proxy_alerts(
        AlertParts::new(Some(vec![AlertingClient::Slack]), Vec::new()),
        Arc::new(FakeUsageCache::new(&journal)),
        Arc::clone(&loud_slack),
        Arc::new(FakeServiceLogging::default()),
    );
    let data = object(&[("model", json("\"gpt-4\""))]);
    loud.start_hanging_request_check(Some(data.clone()));
    drain(&loud.tasks).await;
    assert_eq!(loud_slack.too_long(), vec![Some(data)]);
}

#[rstest]
#[case::absent("{}", None)]
#[case::not_an_object(r#"{"metadata":{"alerting_metadata":"x"}}"#, None)]
#[case::object(r#"{"metadata":{"alerting_metadata":{"team":"t1"}}}"#, Some(("team", "t1")))]
fn alerting_metadata_is_read_from_the_metadata_bucket(
    #[case] text: &str,
    #[case] expected: Option<(&str, &str)>,
) {
    let data = json(text);
    let metadata = alerting_metadata_from(data.as_object().expect("object"));
    match expected {
        Some((name, value)) => assert_eq!(metadata.get(name), Some(&json(&format!("\"{value}\"")))),
        None => assert!(metadata.is_empty()),
    }
}
