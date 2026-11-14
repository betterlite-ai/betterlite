use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::decisions::Json;
use rstest::rstest;
use serde_json::Number;

use super::super::destination::{AlertDestination, SLACK_WEBHOOK_URL_ENV};
use super::super::{Alert, AlertLevel};
use super::{fixture, slack_fixture};

#[tokio::test]
async fn alert_messages_carry_the_type_level_timestamp_and_message() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    let pending = fixture.delivery.queue.pending();
    assert_eq!(
        pending[0].payload.text,
        "Alert type: `llm_exceptions`\nLevel: `High`\nTimestamp: `12:00:00`\n\nMessage: boom"
    );
}

async fn assert_report_message(alert_type: AlertType, message: &str) {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(Some(vec![AlertDestination::Slack]), vec![alert_type]);
    assert!(
        dispatcher
            .send_alert(fixture.alert(message, alert_type))
            .await
            .is_ok()
    );
    let pending = fixture.delivery.queue.pending();
    assert_eq!(
        pending[0].payload.text,
        format!("Alert type: `{alert_type}`{message}")
    );
}

#[tokio::test]
async fn daily_reports_omit_the_level_and_timestamp() {
    assert_report_message(AlertType::DailyReports, " the report").await;
}

#[tokio::test]
async fn new_model_added_omits_the_level_and_timestamp() {
    assert_report_message(AlertType::NewModelAdded, " the model").await;
}

#[tokio::test]
async fn kwargs_are_appended_as_raw_key_value_pairs() {
    let mut fixture = slack_fixture();
    fixture.kwargs.insert(
        "🪢 Langfuse Trace".to_owned(),
        Json::Text("https://trace.example".to_owned()),
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    let pending = fixture.delivery.queue.pending();
    assert!(
        pending[0]
            .payload
            .text
            .contains("\n\n🪢 Langfuse Trace: `https://trace.example`\n\n")
    );
}

#[tokio::test]
async fn alerting_metadata_is_appended_under_its_header() {
    let mut fixture = slack_fixture();
    fixture
        .metadata
        .insert("deprecated_count".to_owned(), Json::Number(Number::from(3)));
    fixture
        .metadata
        .insert("team".to_owned(), Json::Text("alpha".to_owned()));
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    let pending = fixture.delivery.queue.pending();
    let text = &pending[0].payload.text;
    assert!(text.contains("\n\n*Alerting Metadata*: \ndeprecated_count: `3`\n\n"));
    assert!(text.contains("\n\n*Alerting Metadata*: \nteam: `alpha`\n\n"));
}

#[tokio::test]
async fn the_message_orders_the_header_kwargs_metadata_and_footer() {
    let mut fixture = fixture(
        &[
            (SLACK_WEBHOOK_URL_ENV, "https://hooks/slack"),
            ("PROXY_BASE_URL", "https://proxy.example"),
        ],
        512,
    );
    fixture.kwargs.insert(
        "🪢 Langfuse Trace".to_owned(),
        Json::Text("https://trace.example".to_owned()),
    );
    fixture
        .metadata
        .insert("deprecated_count".to_owned(), Json::Number(Number::from(3)));
    fixture
        .metadata
        .insert("team".to_owned(), Json::Text("alpha".to_owned()));
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    assert!(sent.is_ok());
    let pending = fixture.delivery.queue.pending();
    assert_eq!(
        pending[0].payload.text,
        "Alert type: `llm_exceptions`\nLevel: `High`\nTimestamp: `12:00:00`\n\nMessage: boom\n\n🪢 Langfuse Trace: `https://trace.example`\n\n\n\n*Alerting Metadata*: \ndeprecated_count: `3`\n\n\n\n*Alerting Metadata*: \nteam: `alpha`\n\n\n\nProxy URL: `https://proxy.example`"
    );
}

#[rstest]
#[case::text(Json::Text("alpha".to_owned()), "name: `alpha`")]
#[case::number(Json::Number(Number::from(3)), "name: `3`")]
#[case::boolean(Json::Bool(true), "name: `True`")]
#[case::null(Json::Null, "name: `None`")]
#[case::list(
    Json::List(vec![Json::Text("alpha".to_owned()), Json::Text("beta".to_owned())]),
    "name: `[\"alpha\",\"beta\"]`"
)]
#[tokio::test]
async fn alerting_metadata_values_render_their_text(#[case] value: Json, #[case] expected: &str) {
    let mut fixture = slack_fixture();
    fixture.metadata.insert("name".to_owned(), value);
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    let pending = fixture.delivery.queue.pending();
    assert!(pending[0].payload.text.contains(expected));
}

#[tokio::test]
async fn the_proxy_base_url_is_appended_when_set() {
    let fixture = fixture(
        &[
            (SLACK_WEBHOOK_URL_ENV, "https://hooks/slack"),
            ("PROXY_BASE_URL", "https://proxy.example"),
        ],
        512,
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    let pending = fixture.delivery.queue.pending();
    assert!(
        pending[0]
            .payload
            .text
            .ends_with("\n\nProxy URL: `https://proxy.example`")
    );
}

#[tokio::test]
async fn an_empty_proxy_base_url_still_appends_the_footer() {
    let fixture = fixture(
        &[
            (SLACK_WEBHOOK_URL_ENV, "https://hooks/slack"),
            ("PROXY_BASE_URL", ""),
        ],
        512,
    );
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    let pending = fixture.delivery.queue.pending();
    assert!(pending[0].payload.text.ends_with("\n\nProxy URL: ``"));
}

#[tokio::test]
async fn an_unset_proxy_base_url_adds_no_footer() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    let _sent = dispatcher
        .send_alert(fixture.alert("boom", AlertType::LlmExceptions))
        .await;
    let pending = fixture.delivery.queue.pending();
    assert!(!pending[0].payload.text.contains("Proxy URL:"));
}

#[tokio::test]
async fn every_level_renders_its_name() {
    let fixture = slack_fixture();
    let dispatcher = fixture.dispatcher(
        Some(vec![AlertDestination::Slack]),
        vec![AlertType::LlmExceptions],
    );
    for (level, expected) in [
        (AlertLevel::Low, "Level: `Low`"),
        (AlertLevel::Medium, "Level: `Medium`"),
        (AlertLevel::High, "Level: `High`"),
    ] {
        let alert = Alert {
            message: "boom",
            level,
            alert_type: AlertType::LlmExceptions,
            alerting_metadata: &fixture.metadata,
            user_info: None,
            request_model: None,
            api_base: None,
            kwargs: &fixture.kwargs,
        };
        assert!(dispatcher.send_alert(alert).await.is_ok());
        let pending = fixture.delivery.queue.pending();
        let item = pending.last().expect("the alert is enqueued");
        assert!(item.payload.text.contains(expected));
    }
}
