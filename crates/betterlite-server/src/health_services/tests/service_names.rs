use axum::http::StatusCode;
use rstest::rstest;

use crate::health_services::service_name::{ServiceName, unknown_service_message};

use super::support::{World, master_context};

const ACCEPTED: &str = "['slack_budget_alerts', 'email', 'langfuse', 'langfuse_otel', 'slack', 'ms_teams', 'openmeter', 'webhook', 'braintrust', 'otel', 'custom_callback_api', 'langsmith', 'datadog', 'datadog_metrics', 'datadog_llm_observability', 'generic_api', 'arize', 'galileo', 'newrelic', 'pointfive', 'signoz', 'sqs']";

#[rstest]
#[case::slack_budget_alerts("slack_budget_alerts", ServiceName::SlackBudgetAlerts)]
#[case::email("email", ServiceName::Email)]
#[case::langfuse("langfuse", ServiceName::Langfuse)]
#[case::langfuse_otel("langfuse_otel", ServiceName::LangfuseOtel)]
#[case::slack("slack", ServiceName::Slack)]
#[case::ms_teams("ms_teams", ServiceName::MsTeams)]
#[case::openmeter("openmeter", ServiceName::OpenMeter)]
#[case::webhook("webhook", ServiceName::Webhook)]
#[case::braintrust("braintrust", ServiceName::Braintrust)]
#[case::otel("otel", ServiceName::Otel)]
#[case::custom_callback_api("custom_callback_api", ServiceName::CustomCallbackApi)]
#[case::langsmith("langsmith", ServiceName::Langsmith)]
#[case::datadog("datadog", ServiceName::Datadog)]
#[case::datadog_metrics("datadog_metrics", ServiceName::DatadogMetrics)]
#[case::datadog_llm_observability(
    "datadog_llm_observability",
    ServiceName::DatadogLlmObservability
)]
#[case::generic_api("generic_api", ServiceName::GenericApi)]
#[case::arize("arize", ServiceName::Arize)]
#[case::galileo("galileo", ServiceName::Galileo)]
#[case::newrelic("newrelic", ServiceName::NewRelic)]
#[case::pointfive("pointfive", ServiceName::PointFive)]
#[case::signoz("signoz", ServiceName::Signoz)]
#[case::sqs("sqs", ServiceName::Sqs)]
fn service_names_parse_and_round_trip(#[case] text: &str, #[case] name: ServiceName) {
    let parsed = text
        .parse::<ServiceName>()
        .expect("the accepted name parses");
    assert_eq!(parsed, name);
    assert_eq!(name.as_str(), text);
}

#[rstest]
#[case::unknown("nope")]
#[case::case_sensitive("Datadog")]
#[case::empty("")]
#[case::python_repr("['datadog']")]
fn service_other_text_is_not_a_name(#[case] text: &str) {
    assert!(text.parse::<ServiceName>().is_err());
    assert_eq!(
        unknown_service_message(text),
        format!("Service must be in list. Service={text} not in {ACCEPTED}")
    );
}

#[tokio::test(start_paused = true)]
async fn service_endpoint_rejects_an_unknown_name_with_the_proxy_body() {
    let world = World::default();
    let reply = world
        .get("/health/services?service=nope", Some(&master_context()))
        .await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    let error = reply.error();
    assert_eq!(error.error.kind, "auth_error");
    assert_eq!(error.error.param.as_deref(), Some("None"));
    assert_eq!(error.error.code, "400");
    assert_eq!(
        error.error.message,
        format!("{{'error': 'Service must be in list. Service=nope not in {ACCEPTED}'}}")
    );
}
