#![cfg(test)]

use std::str::from_utf8;
use std::time::Duration;

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use http::Method;
use http::header::{AUTHORIZATION, CONTENT_TYPE};
use rstest::rstest;

use crate::credential_sources::getters::VertexParams;
use crate::partner_count::call::{PartnerCountOverrides, PartnerCountRequest, count_tokens};
use crate::partner_count::error::PartnerCountError;
use crate::partner_count::execute::{PartnerCountCall, handle_count_tokens_request};
use crate::partner_count::options::PartnerCountOptions;
use crate::partner_count::plan::DEFAULT_COUNT_TOKENS_LOCATION;
use crate::partner_count::response::{PartnerCount, TOKENIZER_USED};

use super::support::{
    FixedMint, RecordingMint, Script, body, credentials, json, options, params, settings,
};

const CLAUDE: &str = "claude-sonnet-4-6";

fn count_url(project: &str, location: &str) -> String {
    format!(
        "https://{location}-aiplatform.googleapis.com/v1/projects/{project}/locations/{location}/publishers/anthropic/models/count-tokens:rawPredict"
    )
}

fn overrides<'call>(
    project: Option<&'call str>,
    location: Option<&'call str>,
    credentials: Option<&'call ConfiguredCredentials>,
) -> PartnerCountOverrides<'call> {
    PartnerCountOverrides {
        project,
        location,
        credentials,
    }
}

fn wrapper_request<'call>(
    options: &'call PartnerCountOptions,
    settings: &'call VertexSettings,
    params: VertexParams<'call>,
    overrides: PartnerCountOverrides<'call>,
) -> PartnerCountRequest<'call> {
    PartnerCountRequest {
        model: "claude-sonnet-4-6@default",
        messages: &[],
        params,
        options,
        settings,
        api_base: None,
        overrides,
    }
}

#[tokio::test]
async fn posts_the_count_with_the_stripped_model() {
    let transport = Script::answering(200, r#"{"input_tokens": 42}"#);
    let body = body(
        Some("claude-sonnet-4-6@20251001"),
        Some(vec![json(r#"{"role":"user","content":"hi"}"#)]),
    );
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: "claude-sonnet-4-6@default",
        body: &body,
        api_base: None,
        params: params(Some("deployment-project"), None, None),
        options: &options,
        settings: &settings,
    };

    let count = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap();

    assert_eq!(count.input_tokens, 42);
    let sent = transport.sent();
    assert_eq!(sent.len(), 1);
    let request = &sent[0];
    assert_eq!(request.method, Method::POST);
    assert_eq!(
        request.url,
        count_url("deployment-project", DEFAULT_COUNT_TOKENS_LOCATION)
    );
    assert_eq!(request.deadline, Duration::from_secs(30));
    assert_eq!(
        json(from_utf8(&request.body).unwrap()),
        json(r#"{"model": "claude-sonnet-4-6", "messages": [{"role": "user", "content": "hi"}]}"#)
    );
}

#[tokio::test]
async fn the_count_post_carries_a_sensitive_bearer_and_json_content_type() {
    let transport = Script::answering(200, r#"{"input_tokens": 42}"#);
    let body = body(Some("claude-sonnet-4-6@20251001"), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: "claude-sonnet-4-6@default",
        body: &body,
        api_base: None,
        params: params(Some("deployment-project"), None, None),
        options: &options,
        settings: &settings,
    };

    let count = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap();

    assert_eq!(
        count,
        PartnerCount {
            input_tokens: 42,
            tokenizer_used: TOKENIZER_USED,
        }
    );
    let sent = transport.sent();
    let authorization = sent[0].headers.get(AUTHORIZATION).unwrap();
    assert_eq!(authorization.to_str().unwrap(), "Bearer minted");
    assert!(authorization.is_sensitive());
    assert_eq!(
        sent[0].headers.get(CONTENT_TYPE).unwrap().to_str().unwrap(),
        "application/json"
    );
}

#[tokio::test]
async fn a_non_claude_model_also_defaults_to_us_east5() {
    let transport = Script::answering(200, "{}");
    let body = body(Some("mistral-large-2411"), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: "mistral-large-2411",
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap();

    assert_eq!(
        transport.sent()[0].url,
        "https://us-east5-aiplatform.googleapis.com/v1/projects/proj/locations/us-east5/publishers/mistralai/models/count-tokens:rawPredict"
    );
}

#[tokio::test]
async fn a_custom_api_base_from_the_request_reaches_the_endpoint() {
    let transport = Script::answering(200, "{}");
    let options = options(None);
    let settings = settings(None);
    let mut request = wrapper_request(
        &options,
        &settings,
        params(Some("proj"), None, None),
        overrides(None, None, None),
    );
    request.api_base = Some("https://proxy.example/v1");

    count_tokens(&transport, &request, &FixedMint)
        .await
        .unwrap();

    assert_eq!(
        transport.sent()[0].url,
        "https://proxy.example/v1/v1/projects/proj/locations/us-east5/publishers/anthropic/models/count-tokens:rawPredict"
    );
}

#[test]
fn the_tokenizer_name_is_reported_as_vertex_partner_models() {
    assert_eq!(TOKENIZER_USED, "vertex_ai_partner_models");
}

#[tokio::test]
async fn the_model_is_stripped_before_the_publisher_is_chosen() {
    let transport = Script::answering(200, "{}");
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: "gemini-2.5-pro@claude",
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert!(matches!(
        &error,
        PartnerCountError::UnknownPublisher { model } if model == "gemini-2.5-pro"
    ));
    assert!(transport.sent().is_empty());
}

#[tokio::test]
async fn an_empty_messages_list_is_accepted() {
    let transport = Script::answering(200, "{}");
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let count = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap();

    assert_eq!(count.input_tokens, 0);
    assert_eq!(
        json(from_utf8(&transport.sent()[0].body).unwrap()),
        json(r#"{"model": "claude-sonnet-4-6", "messages": []}"#)
    );
}

#[tokio::test]
async fn a_body_without_messages_fails_before_any_request() {
    let transport = Script::answering(200, "{}");
    let body = body(Some(CLAUDE), None);
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert!(matches!(error, PartnerCountError::MissingMessages));
    assert_eq!(error.to_string(), "messages required for token counting");
    assert!(transport.sent().is_empty());
}

#[tokio::test]
async fn a_missing_messages_list_wins_over_an_unknown_publisher() {
    let transport = Script::answering(200, "{}");
    let body = body(Some("gemini-2.5-pro"), None);
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: "gemini-2.5-pro",
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert!(matches!(error, PartnerCountError::MissingMessages));
    assert!(transport.sent().is_empty());
}

#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::bad_request(400)]
#[case::not_found(404)]
#[case::rate_limited(429)]
#[case::server_error(500)]
#[tokio::test]
async fn only_a_200_reply_becomes_a_token_count(#[case] status: u16) {
    let transport = Script::answering(status, "the provider refused");
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert_eq!(error.status_code(), Some(status));
    assert_eq!(
        error.to_string(),
        format!("Token counting request failed with status {status}: the provider refused")
    );
}

#[tokio::test]
async fn a_non_utf8_error_body_is_left_out_of_the_message() {
    let transport = Script::answering_bytes(500, vec![0xff, 0xfe]);
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert_eq!(
        error.to_string(),
        "Token counting request failed with status 500: "
    );
}

#[rstest]
#[case::missing("{}", 0)]
#[case::null(r#"{"input_tokens": null}"#, 0)]
#[case::present(r#"{"input_tokens": 7}"#, 7)]
#[case::negative(r#"{"input_tokens": -3}"#, -3)]
#[case::extra_keys(r#"{"input_tokens": 7, "cached_tokens": 2, "model": "m"}"#, 7)]
#[tokio::test]
async fn a_reply_without_input_tokens_counts_zero(#[case] reply: &str, #[case] tokens: i64) {
    let transport = Script::answering(200, reply);
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let count = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap();

    assert_eq!(count.input_tokens, tokens);
    assert_eq!(count.tokenizer_used, TOKENIZER_USED);
}

#[rstest]
#[case::root_array("[]")]
#[case::root_text(r#""7""#)]
#[case::root_null("null")]
#[tokio::test]
async fn a_reply_root_that_is_not_an_object_fails(#[case] reply: &str) {
    let transport = Script::answering(200, reply);
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert!(matches!(error, PartnerCountError::NotAnObject));
}

#[rstest]
#[case::text(r#"{"input_tokens": "7"}"#)]
#[case::boolean(r#"{"input_tokens": true}"#)]
#[case::float(r#"{"input_tokens": 7.5}"#)]
#[case::list(r#"{"input_tokens": [7]}"#)]
#[case::beyond_i64(r#"{"input_tokens": 9223372036854775808}"#)]
#[tokio::test]
async fn input_tokens_that_are_not_integers_are_refused(#[case] reply: &str) {
    let transport = Script::answering(200, reply);
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert!(matches!(error, PartnerCountError::InputTokens));
}

#[tokio::test]
async fn a_reply_that_is_not_json_fails() {
    let transport = Script::answering(200, "not json");
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert!(matches!(error, PartnerCountError::Decode(_)));
}

#[tokio::test]
async fn a_closed_transport_keeps_its_source() {
    let transport = Script::default();
    let body = body(Some(CLAUDE), Some(Vec::new()));
    let options = options(None);
    let settings = settings(None);
    let call = PartnerCountCall {
        model: CLAUDE,
        body: &body,
        api_base: None,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
    };

    let error = handle_count_tokens_request(&transport, &call, &FixedMint)
        .await
        .unwrap_err();

    assert!(matches!(&error, PartnerCountError::Transport(_)));
    assert_eq!(error.status_code(), None);
}

#[tokio::test]
async fn the_wrapper_sends_only_the_model_and_messages() {
    let transport = Script::answering(200, r#"{"input_tokens": 3}"#);
    let options = options(None);
    let settings = settings(None);
    let messages = [json(r#"{"role": "user", "content": "hi"}"#)];
    let request = PartnerCountRequest {
        model: "claude-sonnet-4-6@default",
        messages: &messages,
        params: params(Some("proj"), None, None),
        options: &options,
        settings: &settings,
        api_base: None,
        overrides: overrides(None, None, None),
    };

    let count = count_tokens(&transport, &request, &FixedMint)
        .await
        .unwrap();

    assert_eq!(count.input_tokens, 3);
    assert_eq!(
        json(from_utf8(&transport.sent()[0].body).unwrap()),
        json(r#"{"model": "claude-sonnet-4-6", "messages": [{"role": "user", "content": "hi"}]}"#)
    );
}

#[tokio::test]
async fn explicit_settings_overlay_the_deployment_copy() {
    let transport = Script::answering(200, "{}");
    let options = options(None);
    let settings = settings(Some("settings-location"));
    let deployment = credentials("deployment");
    let explicit = credentials("explicit");
    let mint = RecordingMint::default();
    let request = wrapper_request(
        &options,
        &settings,
        params(
            Some("deployment-project"),
            Some("deployment-location"),
            Some(&deployment),
        ),
        overrides(
            Some("explicit-project"),
            Some("us-central1"),
            Some(&explicit),
        ),
    );

    count_tokens(&transport, &request, &mint).await.unwrap();

    assert_eq!(mint.seen(), vec![Some("explicit".to_owned())]);
    assert_eq!(
        transport.sent()[0].url,
        count_url("explicit-project", "us-central1")
    );
}

#[tokio::test]
async fn the_count_tokens_location_beats_every_explicit_location() {
    let transport = Script::answering(200, "{}");
    let options = options(Some("asia-southeast1"));
    let settings = settings(Some("settings-location"));
    let mint = RecordingMint::default();
    let request = wrapper_request(
        &options,
        &settings,
        params(Some("proj"), Some("deployment-location"), None),
        overrides(None, Some("us-central1"), None),
    );

    count_tokens(&transport, &request, &mint).await.unwrap();

    assert_eq!(
        transport.sent()[0].url,
        count_url("proj", "asia-southeast1")
    );
}

#[tokio::test]
async fn the_minted_project_is_used_when_none_is_configured() {
    let transport = Script::answering(200, "{}");
    let options = options(None);
    let settings = settings(None);
    let mint = RecordingMint::default();
    let request = wrapper_request(
        &options,
        &settings,
        params(None, None, None),
        overrides(None, None, None),
    );

    count_tokens(&transport, &request, &mint).await.unwrap();

    assert_eq!(mint.projects(), vec![None::<String>]);
    assert!(
        transport.sent()[0]
            .url
            .contains("/v1/projects/minted-project/")
    );
}

#[tokio::test]
async fn a_count_leaves_the_deployment_params_usable() {
    let transport = Script::answering(200, "{}");
    let options = options(None);
    let settings = settings(Some("settings-location"));
    let deployment = credentials("deployment");
    let explicit = credentials("explicit");
    let mint = RecordingMint::default();
    let deployment_params = params(
        Some("deployment-project"),
        Some("deployment-location"),
        Some(&deployment),
    );
    let explicit_overrides = overrides(
        Some("explicit-project"),
        Some("us-central1"),
        Some(&explicit),
    );
    let no_overrides = overrides(None, None, None);
    let overlaid = wrapper_request(&options, &settings, deployment_params, explicit_overrides);
    let plain = wrapper_request(&options, &settings, deployment_params, no_overrides);

    count_tokens(&transport, &overlaid, &mint).await.unwrap();
    count_tokens(&transport, &plain, &mint).await.unwrap();

    assert_eq!(mint.seen()[1].as_deref(), Some("deployment"));
    let sent = transport.sent();
    assert_eq!(sent[0].url, count_url("explicit-project", "us-central1"));
    assert_eq!(
        sent[1].url,
        count_url("deployment-project", "deployment-location")
    );
}

#[tokio::test]
async fn an_empty_explicit_setting_keeps_the_deployment_value() {
    let transport = Script::answering(200, "{}");
    let options = options(None);
    let settings = settings(None);
    let deployment = credentials("deployment");
    let mint = RecordingMint::default();
    let request = wrapper_request(
        &options,
        &settings,
        params(
            Some("deployment-project"),
            Some("deployment-location"),
            Some(&deployment),
        ),
        overrides(Some(""), Some(""), None),
    );

    count_tokens(&transport, &request, &mint).await.unwrap();

    assert_eq!(mint.seen(), vec![Some("deployment".to_owned())]);
    assert_eq!(
        transport.sent()[0].url,
        count_url("deployment-project", "deployment-location")
    );
}

#[tokio::test]
async fn a_status_failure_keeps_its_status_through_the_wrapper() {
    let transport = Script::answering(404, "no such model");
    let options = options(None);
    let settings = settings(None);
    let request = wrapper_request(
        &options,
        &settings,
        params(Some("proj"), None, None),
        overrides(None, None, None),
    );

    let error = count_tokens(&transport, &request, &FixedMint)
        .await
        .unwrap_err();

    assert_eq!(error.status_code, 404);
    assert_eq!(
        error.message,
        "Token counting request failed with status 404: no such model"
    );
    assert_eq!(error.headers, None);
}

#[tokio::test]
async fn other_failures_become_a_vertex_500() {
    let transport = Script::default();
    let options = options(None);
    let settings = settings(None);
    let request = wrapper_request(
        &options,
        &settings,
        params(Some("proj"), None, None),
        overrides(None, None, None),
    );

    let error = count_tokens(&transport, &request, &FixedMint)
        .await
        .unwrap_err();

    assert_eq!(error.status_code, 500);
    assert_eq!(error.headers, None);
}
