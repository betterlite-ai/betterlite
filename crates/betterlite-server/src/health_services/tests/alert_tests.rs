use axum::http::StatusCode;
use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_logging::alert_dispatch::AlertLevel;
use betterlite_logging::budget_email::event::{EntityType, WebhookEvent, WebhookEventType};
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::ExposeSecret as _;

use crate::health_services::alert_tests::{TeamsDelivery, UserBudgetAlert};

use super::support::{
    ADMIN_KEY, FakeAlerts, USER_KEY, World, key_context, master_context, user_context,
    viewer_context,
};

const EMAIL_KEY_ALIAS: &str =
    "Email Test key (This is only a test alert key. DO NOT USE THIS IN PRODUCTION.)";

#[tokio::test(start_paused = true)]
async fn alert_webhook_requires_a_proxy_admin() {
    let world = World::default();
    let reply = world
        .get("/health/services?service=webhook", Some(&user_context()))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.param.as_deref(), Some("None"));
    assert_eq!(error.error.code, "403");
    assert_eq!(
        error.error.message,
        "{'error': 'Only proxy admins can trigger the webhook test alert.'}"
    );
    assert!(world.alerts.budget_tests().is_empty());
}

#[tokio::test(start_paused = true)]
async fn alert_webhook_admin_sends_the_budget_test_and_answers_null() {
    let world = World::default();
    let reply = world
        .get(
            "/health/services?service=webhook",
            Some(&key_context(ADMIN_KEY)),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "null");
    assert_eq!(
        world.alerts.budget_tests(),
        vec![UserBudgetAlert {
            token: "hashed-admin-key".to_owned(),
            user_id: Some("admin-user".to_owned()),
            key_alias: Some("admin-alias".to_owned()),
            team_id: Some("admin-team".to_owned()),
        }]
    );
}

#[tokio::test(start_paused = true)]
async fn alert_webhook_view_only_admin_passes() {
    let world = World::default();
    let reply = world
        .get("/health/services?service=webhook", Some(&viewer_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.text(), "null");
    assert_eq!(
        world.alerts.budget_tests(),
        vec![UserBudgetAlert {
            token: String::new(),
            user_id: None,
            key_alias: None,
            team_id: None,
        }]
    );
}

#[rstest]
#[case::slack("slack")]
#[case::slack_budget_alerts("slack_budget_alerts")]
#[tokio::test(start_paused = true)]
async fn alert_slack_requires_slack_in_the_alerting_list(#[case] service: &str) {
    let world = World::default();
    let reply = world
        .get(
            &format!("/health/services?service={service}"),
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.param.as_deref(), Some("None"));
    assert_eq!(error.error.code, "422");
    assert_eq!(
        error.error.message,
        format!(
            "{{'error': '\"{service}\" not in proxy config: general_settings. Unable to test this.'}}"
        )
    );
}

#[tokio::test(start_paused = true)]
async fn alert_slack_budget_alerts_name_runs_with_slack_alerting() {
    let world = World::default().with_alerting(&["slack"]);
    let reply = world
        .get(
            "/health/services?service=slack_budget_alerts",
            Some(&master_context()),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let slack = reply.slack();
    assert_eq!(slack.status, "success");
    assert!(slack.alert_types.is_empty());
    assert_eq!(
        slack.message,
        "Mock Slack Alert sent, verify Slack Alert Received on your channel"
    );
    let sent = world.alerts.sent_alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, "This is a test slack alert message");
    assert_eq!(sent[0].level, AlertLevel::Low);
    assert_eq!(sent[0].alert_type, AlertType::BudgetAlerts);
}

#[rstest]
#[case::llm_exceptions(AlertType::LlmExceptions, "LLM Exception test alert")]
#[case::llm_too_slow(AlertType::LlmTooSlow, "LLM Too Slow test alert")]
#[case::llm_requests_hanging(AlertType::LlmRequestsHanging, "LLM Requests Hanging test alert")]
#[case::budget_alerts(AlertType::BudgetAlerts, "Budget Alert test alert")]
#[case::db_exceptions(AlertType::DbExceptions, "DB Exception test alert")]
#[case::outage_alerts(AlertType::OutageAlerts, "Outage Alert Exception test alert")]
#[case::daily_reports(AlertType::DailyReports, "Daily Reports test alert")]
#[case::other_type_gets_the_budget_text(AlertType::SpendReports, "Budget Alert test alert")]
#[case::anomalies_get_the_budget_text(AlertType::UserSpendAnomalies, "Budget Alert test alert")]
#[tokio::test(start_paused = true)]
async fn alert_slack_sends_one_alert_per_configured_type(
    #[case] alert_type: AlertType,
    #[case] message: &str,
) {
    let world = World::default().with_alerting(&["slack"]).with_alerts(
        FakeAlerts::default()
            .with_webhook_alert_types(&[alert_type])
            .with_active_alert_types(&[alert_type]),
    );
    let reply = world
        .get("/health/services?service=slack", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.slack().alert_types, vec![alert_type.to_string()]);
    let sent = world.alerts.sent_alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].message, message);
    assert_eq!(sent[0].level, AlertLevel::Low);
    assert_eq!(sent[0].alert_type, alert_type);
}

#[tokio::test(start_paused = true)]
async fn alert_slack_skips_types_outside_the_active_list() {
    let world = World::default().with_alerting(&["slack"]).with_alerts(
        FakeAlerts::default()
            .with_webhook_alert_types(&[AlertType::BudgetAlerts, AlertType::LlmExceptions])
            .with_active_alert_types(&[AlertType::LlmExceptions]),
    );
    let reply = world
        .get("/health/services?service=slack", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(
        reply.slack().alert_types,
        vec![AlertType::LlmExceptions.to_string()]
    );
    let sent = world.alerts.sent_alerts();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].alert_type, AlertType::LlmExceptions);
    assert_eq!(sent[0].message, "LLM Exception test alert");
}

#[tokio::test(start_paused = true)]
async fn alert_slack_sends_every_type_when_no_active_list_is_set() {
    let world = World::default().with_alerting(&["slack"]).with_alerts(
        FakeAlerts::default()
            .with_webhook_alert_types(&[AlertType::BudgetAlerts, AlertType::LlmExceptions]),
    );
    let reply = world
        .get("/health/services?service=slack", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert!(reply.slack().alert_types.is_empty());
    let sent = world.alerts.sent_alerts();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[0].alert_type, AlertType::BudgetAlerts);
    assert_eq!(sent[0].message, "Budget Alert test alert");
    assert_eq!(sent[1].alert_type, AlertType::LlmExceptions);
    assert_eq!(sent[1].message, "LLM Exception test alert");
}

#[rstest]
#[case::without_database(false, &[])]
#[case::with_database(true, &["monthly", "weekly"])]
#[tokio::test(start_paused = true)]
async fn alert_slack_sends_spend_reports_only_with_a_database(
    #[case] has_database: bool,
    #[case] expected: &[&str],
) {
    let world = World::default()
        .with_alerting(&["slack"])
        .with_alerts(FakeAlerts::default().with_database(has_database));
    let reply = world
        .get("/health/services?service=slack", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(world.alerts.reports(), expected.to_vec());
}

#[tokio::test(start_paused = true)]
async fn alert_ms_teams_requires_the_alerting_entry() {
    let world = World::default()
        .with_alerts(FakeAlerts::default().with_teams_url(Some("https://teams.example/hook")));
    let reply = world
        .get("/health/services?service=ms_teams", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error = reply.error();
    assert_eq!(error.error.code, "422");
    assert_eq!(
        error.error.message,
        "{'error': '\"ms_teams\" not in proxy config: general_settings. Unable to test this.'}"
    );
    assert!(world.alerts.teams_posts().is_empty());
}

#[tokio::test(start_paused = true)]
async fn alert_ms_teams_requires_a_webhook_url() {
    let world = World::default().with_alerting(&["ms_teams"]);
    let reply = world
        .get("/health/services?service=ms_teams", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let error = reply.error();
    assert_eq!(error.error.code, "422");
    assert_eq!(
        error.error.message,
        "{'error': 'MS_TEAMS_WEBHOOK_URL not set. Unable to test this.'}"
    );
    assert!(world.alerts.teams_posts().is_empty());
}

#[tokio::test(start_paused = true)]
async fn alert_ms_teams_posts_the_test_message_with_the_clock_time() {
    let world = World::default()
        .with_alerting(&["ms_teams"])
        .with_alerts(FakeAlerts::default().with_teams_url(Some("https://teams.example/hook")));
    let reply = world
        .get("/health/services?service=ms_teams", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let success = reply.success();
    assert_eq!(success.status, "success");
    assert_eq!(
        success.message,
        "Mock MS Teams Alert sent, verify MS Teams Alert Received in your channel"
    );
    assert_eq!(
        world.alerts.teams_posts(),
        vec![(
            "https://teams.example/hook".to_owned(),
            "Alert type: `budget_alerts`\nLevel: `Low`\nTimestamp: `03:04:05`\n\nMessage: This is a test MS Teams alert message"
                .to_owned(),
        )]
    );
}

#[rstest]
#[case::bad_request(400, "bad request")]
#[case::server_error(500, "boom")]
#[tokio::test(start_paused = true)]
async fn alert_ms_teams_delivery_failure_answers_500(#[case] status_code: u16, #[case] body: &str) {
    let world = World::default().with_alerting(&["ms_teams"]).with_alerts(
        FakeAlerts::default()
            .with_teams_url(Some("https://teams.example/hook"))
            .with_teams_delivery(TeamsDelivery {
                status_code,
                body: body.to_owned(),
            }),
    );
    let reply = world
        .get("/health/services?service=ms_teams", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::INTERNAL_SERVER_ERROR);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.param.as_deref(), Some("None"));
    assert_eq!(error.error.code, "500");
    assert_eq!(
        error.error.message,
        format!("{{'error': 'MS Teams webhook returned status {status_code}: {body}'}}")
    );
}

#[tokio::test(start_paused = true)]
async fn alert_ms_teams_status_below_400_succeeds() {
    let world = World::default().with_alerting(&["ms_teams"]).with_alerts(
        FakeAlerts::default()
            .with_teams_url(Some("https://teams.example/hook"))
            .with_teams_delivery(TeamsDelivery {
                status_code: 399,
                body: "slow".to_owned(),
            }),
    );
    let reply = world
        .get("/health/services?service=ms_teams", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.success().status, "success");
}

#[rstest]
#[case::with_an_address(Some("ops@example.com"))]
#[case::without_an_address(None)]
#[tokio::test(start_paused = true)]
async fn alert_email_sends_the_test_alert_event(#[case] address: Option<&str>) {
    let mut world = World::default();
    world.settings.test_email_address = address.map(str::to_owned);
    let reply = world
        .get(
            "/health/services?service=email",
            Some(&key_context(USER_KEY)),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    let success = reply.success();
    assert_eq!(success.status, "success");
    assert_eq!(
        success.message,
        "Mock Email Alert sent, verify Email Alert Received"
    );
    let emails = world.alerts.emails();
    assert_eq!(emails.len(), 1);
    assert_test_email_event(&emails[0], address);
}

fn assert_test_email_event(event: &WebhookEvent, address: Option<&str>) {
    assert_eq!(event.event, WebhookEventType::KeyCreated);
    assert_eq!(event.event_message, "Test Email Alert");
    assert_eq!(event.event_group, EntityType::Key);
    assert_eq!(event.spend, Decimal::ZERO);
    assert_eq!(event.max_budget, Some(Decimal::ZERO));
    assert_eq!(event.soft_budget, None);
    let token = event
        .token
        .as_ref()
        .expect("the test event carries the caller's token");
    assert_eq!(token.expose_secret(), "hashed-user-key");
    assert_test_email_details(event, address);
}

fn assert_test_email_details(event: &WebhookEvent, address: Option<&str>) {
    assert_eq!(event.customer_id, None);
    assert_eq!(event.user_id.as_deref(), Some("user-1"));
    assert_eq!(event.team_id.as_deref(), Some("team-1"));
    assert_eq!(event.team_alias, None);
    assert_eq!(event.organization_id, None);
    assert_eq!(event.user_email.as_deref(), address);
    assert_eq!(event.key_alias.as_deref(), Some(EMAIL_KEY_ALIAS));
    assert_eq!(event.projected_exceeded_date, None);
    assert_eq!(event.projected_spend, None);
    assert_eq!(event.alert_emails, None);
    assert_eq!(event.max_budget_alert_emails, None);
}

#[rstest]
#[case::sent(true)]
#[case::not_sent(false)]
#[tokio::test(start_paused = true)]
async fn alert_email_answers_success_whatever_the_send_result(#[case] sent: bool) {
    let world = World::default()
        .with_test_email("ops@example.com")
        .with_alerts(FakeAlerts::default().with_email_sent(sent));
    let reply = world
        .get(
            "/health/services?service=email",
            Some(&key_context(USER_KEY)),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.success().status, "success");
    assert_eq!(world.alerts.emails().len(), 1);
}
