#![cfg(test)]

use betterlite_config::sections::alert_settings::types::AlertType;
use betterlite_models::api_base_lookup::ApiBaseError;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use crate::alert_dispatch::AlertLevel;
use crate::hanging_requests::tests::support;

use super::callback::{SlowResponseError, slow_response_callback};
use super::deployment::deployment_latencies;
use super::helper::{
    MESSAGE_NOT_LOGGED, MESSAGE_PREVIEW_CHARS, MessagePrivacy, SlowResponseRequest,
    slow_response_parameters,
};
use super::settings::RequestAlertSettings;
use super::wrapper::response_taking_too_long;

const SLOW_MESSAGE: &str =
    "`Responses are slow - 300.25s response time > Alerting threshold: 300.0s`";

fn default_request<'request>(model: &'request str) -> SlowResponseRequest<'request> {
    SlowResponseRequest {
        model,
        litellm_params: None,
        messages: None,
        input: None,
        start_time: 10.0,
        end_time: 310.25,
        privacy: MessagePrivacy::default(),
    }
}

#[test]
fn the_helper_extracts_timing_model_api_base_and_the_preview() {
    let resolver = support::FixedResolver::new(Some("https://api.openai.com"));
    let params = support::object(&[("api_base", support::json("\"https://deployment.example\""))]);
    let messages = support::json("\"hello\"");
    let request = SlowResponseRequest {
        model: "gpt-4",
        litellm_params: Some(&params),
        messages: Some(&messages),
        input: None,
        start_time: 100.0,
        end_time: 105.25,
        privacy: MessagePrivacy::default(),
    };
    let parameters = slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(
        parameters.time_difference_seconds.to_bits(),
        5.25_f64.to_bits()
    );
    assert_eq!(parameters.model, "gpt-4");
    assert_eq!(
        parameters.api_base.as_deref(),
        Some("https://api.openai.com")
    );
    assert_eq!(parameters.messages, "hello");
    assert_eq!(resolver.calls(), vec![("gpt-4".to_owned(), params)]);
}

#[test]
fn the_helper_queries_the_resolver_with_an_empty_map_without_params() {
    let resolver = support::FixedResolver::new(None);
    let request = default_request("gpt-4");
    slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(
        resolver.calls(),
        vec![("gpt-4".to_owned(), IndexMap::new())]
    );
}

#[test]
fn the_helper_falls_back_to_input_when_messages_is_absent() {
    let resolver = support::FixedResolver::new(None);
    let input = support::json("\"from the input\"");
    let mut request = default_request("gpt-4");
    request.input = Some(&input);
    let parameters = slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(parameters.messages, "from the input");
}

#[test]
fn the_helper_falls_back_to_input_when_messages_is_null() {
    let resolver = support::FixedResolver::new(None);
    let messages = Json::Null;
    let input = support::json("\"from the input\"");
    let mut request = default_request("gpt-4");
    request.messages = Some(&messages);
    request.input = Some(&input);
    let parameters = slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(parameters.messages, "from the input");
}

#[test]
fn the_helper_renders_missing_messages_as_null() {
    let resolver = support::FixedResolver::new(None);
    let request = default_request("gpt-4");
    let parameters = slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(parameters.messages, "null");
}

#[test]
fn the_helper_renders_a_structured_message_as_compact_json() {
    let resolver = support::FixedResolver::new(None);
    let messages = support::json(r#"[{"role": "user", "content": "hi"}]"#);
    let mut request = default_request("gpt-4");
    request.messages = Some(&messages);
    let parameters = slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(parameters.messages, r#"[{"role":"user","content":"hi"}]"#);
}

#[rstest]
#[case::exactly_the_preview(100)]
#[case::one_past_the_preview(101)]
#[case::far_past_the_preview(200)]
fn the_helper_truncates_the_preview(#[case] size: usize) {
    let resolver = support::FixedResolver::new(None);
    let messages = Json::Text("é".repeat(size));
    let mut request = default_request("gpt-4");
    request.messages = Some(&messages);
    let parameters = slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(parameters.messages.chars().count(), MESSAGE_PREVIEW_CHARS);
    assert_eq!(parameters.messages, "é".repeat(MESSAGE_PREVIEW_CHARS));
}

#[rstest]
#[case::logging_on(
    MessagePrivacy { turn_off_message_logging: false, redact_messages_in_exceptions: false },
    "hello"
)]
#[case::messages_off(
    MessagePrivacy { turn_off_message_logging: true, redact_messages_in_exceptions: false },
    MESSAGE_NOT_LOGGED
)]
#[case::redaction_on(
    MessagePrivacy { turn_off_message_logging: false, redact_messages_in_exceptions: true },
    MESSAGE_NOT_LOGGED
)]
#[case::both_on(
    MessagePrivacy { turn_off_message_logging: true, redact_messages_in_exceptions: true },
    MESSAGE_NOT_LOGGED
)]
fn the_helper_hides_the_preview(#[case] privacy: MessagePrivacy, #[case] expected: &str) {
    let resolver = support::FixedResolver::new(None);
    let messages = support::json("\"hello\"");
    let mut request = default_request("gpt-4");
    request.messages = Some(&messages);
    request.privacy = privacy;
    let parameters = slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(parameters.messages, expected);
}

#[test]
fn the_helper_redacts_a_long_preview() {
    let resolver = support::FixedResolver::new(None);
    let messages = Json::Text("é".repeat(150));
    let mut request = default_request("gpt-4");
    request.messages = Some(&messages);
    request.privacy = MessagePrivacy {
        turn_off_message_logging: true,
        redact_messages_in_exceptions: false,
    };
    let parameters = slow_response_parameters(&resolver, &request).expect("the helper works");
    assert_eq!(parameters.messages, MESSAGE_NOT_LOGGED);
}

#[test]
fn the_helper_returns_the_resolver_error() {
    let resolver = support::FixedResolver::failing();
    let request = default_request("gpt-4");
    let error = slow_response_parameters(&resolver, &request).expect_err("the resolver fails");
    assert!(matches!(error, ApiBaseError::InvalidVertexLocation { .. }));
}

#[test]
fn deployment_latencies_sort_pairs_into_a_code_block() {
    let metadata = support::metadata(
        r#"{"_latency_per_deployment": {"https://b.example": 0.5, "https://a.example": 0.0133, "https://c.example": 2.0}}"#,
    );
    assert_eq!(
        deployment_latencies(&metadata).as_deref(),
        Some("```\nhttps://a.example: 0.01s\nhttps://b.example: 0.5s\nhttps://c.example: 2.0s```")
    );
}

#[rstest]
#[case::no_key("{}")]
#[case::empty_map(r#"{"_latency_per_deployment": {}}"#)]
#[case::not_a_map(r#"{"_latency_per_deployment": "fast"}"#)]
#[case::non_numeric(r#"{"_latency_per_deployment": {"https://a.example": "fast"}}"#)]
fn deployment_latencies_are_absent_for_each_missing_shape(#[case] text: &str) {
    let metadata = support::metadata(text);
    assert_eq!(deployment_latencies(&metadata), None);
}

#[rstest]
#[case::below_the_threshold(299.5, 0)]
#[case::at_the_threshold(300.0, 0)]
#[case::above_the_threshold(300.5, 1)]
#[tokio::test]
async fn the_callback_sends_only_above_the_threshold(#[case] end_time: f64, #[case] sent: usize) {
    let fixture = support::fixture();
    let mut request = default_request("gpt-4");
    request.start_time = 0.0;
    request.end_time = end_time;
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    assert_eq!(fixture.sink.take().len(), sent);
    assert_eq!(fixture.resolver.calls().len(), 1);
}

#[tokio::test]
async fn the_callback_sends_the_formatted_slow_alert() {
    let fixture = support::fixture();
    let messages = support::json("\"hello\"");
    let mut request = default_request("gpt-4");
    request.messages = Some(&messages);
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    let alerts = fixture.sink.take();
    assert_eq!(alerts.len(), 1);
    assert_eq!(
        alerts[0].message,
        format!(
            "{SLOW_MESSAGE}\nRequest Model: `gpt-4`\nAPI Base: `https://api.openai.com`\nMessages: `hello`"
        )
    );
    assert_eq!(alerts[0].level, AlertLevel::Low);
    assert_eq!(alerts[0].alert_type, AlertType::LlmTooSlow);
    assert_eq!(alerts[0].request_model.as_deref(), Some("gpt-4"));
    assert_eq!(
        alerts[0].api_base.as_deref(),
        Some("https://api.openai.com")
    );
    assert!(alerts[0].alerting_metadata.is_empty());
    assert!(alerts[0].kwargs.is_empty());
}

#[tokio::test]
async fn the_callback_uses_empty_strings_for_a_missing_api_base() {
    let fixture = support::fixture_with(
        support::alert_settings(300.0),
        support::FixedResolver::new(None),
    );
    let request = default_request("gpt-4");
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    let alerts = fixture.sink.take();
    assert!(alerts[0].message.contains("API Base: ``"));
    assert_eq!(alerts[0].api_base, None);
}

#[tokio::test]
async fn the_callback_adds_key_name_team_latencies_and_alerting_metadata() {
    let fixture = support::fixture();
    let metadata = support::object(&[(
        "metadata",
        support::json(
            r#"{
                "user_api_key_alias": "key-1",
                "user_api_key_team_alias": "team-1",
                "alerting_metadata": {"team": "team-1"},
                "_latency_per_deployment": {"https://b.example": 2.0, "https://a.example": 0.0133}
            }"#,
        ),
    )]);
    let messages = support::json("\"hello\"");
    let mut request = default_request("gpt-4");
    request.litellm_params = Some(&metadata);
    request.messages = Some(&messages);
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    let alerts = fixture.sink.take();
    assert_eq!(
        alerts[0].message,
        format!(
            "{SLOW_MESSAGE}\n\nKey Name: `key-1`\nTeam: `team-1`\nRequest Model: `gpt-4`\nAPI Base: `https://api.openai.com`\nMessages: `hello`\nAvailable Deployment Latencies\n```\nhttps://a.example: 0.01s\nhttps://b.example: 2.0s```"
        )
    );
    assert_eq!(
        alerts[0].alerting_metadata,
        support::object(&[("team", support::json("\"team-1\""))])
    );
}

#[tokio::test]
async fn the_callback_omits_the_metadata_block_without_metadata() {
    let fixture = support::fixture();
    let params = support::object(&[("api_base", support::json("\"https://deployment.example\""))]);
    let mut request = default_request("gpt-4");
    request.litellm_params = Some(&params);
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    let alerts = fixture.sink.take();
    assert!(!alerts[0].message.contains("Key Name:"));
    assert!(!alerts[0].message.contains("Available Deployment Latencies"));
    assert!(alerts[0].alerting_metadata.is_empty());
}

#[tokio::test]
async fn the_callback_adds_only_the_key_name_when_the_team_is_absent() {
    let fixture = support::fixture();
    let metadata = support::object(&[(
        "metadata",
        support::json(r#"{"user_api_key_alias": "key-1"}"#),
    )]);
    let mut request = default_request("gpt-4");
    request.litellm_params = Some(&metadata);
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    let alerts = fixture.sink.take();
    assert!(
        alerts[0]
            .message
            .contains("\n\nKey Name: `key-1`\nTeam: ``")
    );
}

#[tokio::test]
async fn the_callback_ignores_metadata_that_is_not_an_object() {
    let fixture = support::fixture();
    let params = support::object(&[("metadata", support::json("\"text\""))]);
    let mut request = default_request("gpt-4");
    request.litellm_params = Some(&params);
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    let alerts = fixture.sink.take();
    assert!(!alerts[0].message.contains("Key Name:"));
    assert!(alerts[0].alerting_metadata.is_empty());
}

#[tokio::test]
async fn the_callback_keeps_a_null_alerting_metadata_empty() {
    let fixture = support::fixture();
    let metadata =
        support::object(&[("metadata", support::json(r#"{"alerting_metadata": null}"#))]);
    let mut request = default_request("gpt-4");
    request.litellm_params = Some(&metadata);
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    let alerts = fixture.sink.take();
    assert!(alerts[0].alerting_metadata.is_empty());
}

#[tokio::test]
async fn the_callback_does_nothing_without_alerting() {
    let fixture = support::fixture_with(
        RequestAlertSettings {
            alerting_configured: false,
            alert_types: vec![AlertType::LlmTooSlow, AlertType::LlmRequestsHanging],
            alerting_threshold_seconds: 300.0,
        },
        support::FixedResolver::new(Some("https://api.openai.com")),
    );
    let request = default_request("gpt-4");
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    assert!(fixture.sink.take().is_empty());
    assert!(fixture.resolver.calls().is_empty());
}

#[tokio::test]
async fn the_callback_rechecks_the_settings_on_each_call() {
    let fixture = support::fixture();
    let request = default_request("gpt-4");
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    assert_eq!(fixture.sink.take().len(), 1);
    fixture.settings.set(RequestAlertSettings {
        alerting_configured: false,
        alert_types: vec![AlertType::LlmTooSlow, AlertType::LlmRequestsHanging],
        alerting_threshold_seconds: 300.0,
    });
    slow_response_callback(&fixture.check, &request)
        .await
        .expect("the callback works");
    assert!(fixture.sink.take().is_empty());
}

#[tokio::test]
async fn the_callback_returns_the_resolver_error() {
    let fixture = support::fixture_with(
        support::alert_settings(300.0),
        support::FixedResolver::failing(),
    );
    let request = default_request("gpt-4");
    let error = slow_response_callback(&fixture.check, &request)
        .await
        .expect_err("the resolver fails");
    assert!(matches!(
        error,
        SlowResponseError::ApiBase(ApiBaseError::InvalidVertexLocation { .. })
    ));
}

#[rstest]
#[case::selected(
    true,
    vec![AlertType::LlmTooSlow, AlertType::LlmRequestsHanging],
    1
)]
#[case::hanging_not_selected(true, vec![AlertType::LlmTooSlow], 0)]
#[case::alerting_off(false, vec![AlertType::LlmRequestsHanging], 0)]
#[tokio::test]
async fn the_wrapper_adds_the_request_only_when_alerting_and_hanging_are_selected(
    #[case] configured: bool,
    #[case] alert_types: Vec<AlertType>,
    #[case] cached: usize,
) {
    let fixture = support::fixture_with(
        RequestAlertSettings {
            alerting_configured: configured,
            alert_types,
            alerting_threshold_seconds: 300.0,
        },
        support::FixedResolver::new(None),
    );
    let data = support::object(&[
        ("model", support::json("\"gpt-4\"")),
        ("litellm_call_id", support::json("\"call-1\"")),
        ("deployment", support::json(r#"{"litellm_params": {}}"#)),
    ]);
    response_taking_too_long(&fixture.check, Some(&data))
        .await
        .expect("the wrapper works");
    assert_eq!(fixture.cache.entry_count(), cached);
    assert_eq!(fixture.resolver.calls().len(), cached);
}

#[tokio::test]
async fn the_wrapper_ignores_absent_request_data() {
    let fixture = support::fixture();
    response_taking_too_long(&fixture.check, None)
        .await
        .expect("the wrapper works");
    assert_eq!(fixture.cache.entry_count(), 0);
}

#[tokio::test]
async fn the_wrapper_returns_a_field_error() {
    let fixture = support::fixture();
    let data = support::object(&[
        ("model", support::json("5")),
        ("litellm_call_id", support::json("\"call-1\"")),
    ]);
    let error = response_taking_too_long(&fixture.check, Some(&data))
        .await
        .expect_err("a number is not a model");
    assert_eq!(error.to_string(), "the model field must be a string");
    assert_eq!(fixture.cache.entry_count(), 0);
}
