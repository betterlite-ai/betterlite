#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_models::decisions::openai::ChoiceValue;
use betterlite_models::decisions::{DecisionsJson, Extra, Json, ir};
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;

use super::{
    API_BASE_ENV, API_KEY_ENV, PLACEHOLDER_KEY, decisions::HostedVllmDecisionsConfig, image_edit,
    resolved_api_base, resolved_api_key, responses, transcription, versioned_url,
};
use crate::decisions::transformation::provider_config::DecisionsProviderConfig;

fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn headers(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

// the explicit key wins when it is non-empty, and an empty one falls through to the environment
#[test]
fn resolved_api_key_reads_the_explicit_key_first() {
    let configured = environment(&[(API_KEY_ENV, "env-key")]);
    assert_eq!(
        resolved_api_key(Some(&secret("explicit")), &configured),
        "explicit"
    );
}

#[test]
fn resolved_api_key_falls_through_an_empty_explicit_key() {
    let configured = environment(&[(API_KEY_ENV, "env-key")]);
    assert_eq!(resolved_api_key(Some(&secret("")), &configured), "env-key");
}

#[test]
fn resolved_api_key_ignores_an_empty_environment_value() {
    let configured = environment(&[(API_KEY_ENV, "")]);
    assert_eq!(resolved_api_key(None, &configured), PLACEHOLDER_KEY);
}

#[test]
fn resolved_api_key_uses_the_placeholder_without_a_key() {
    assert_eq!(
        resolved_api_key(None, &environment(&[])),
        String::from("fake-api-key")
    );
}

#[test]
fn resolved_api_base_prefers_the_explicit_base() {
    let configured = environment(&[(API_BASE_ENV, "http://env")]);
    assert_eq!(
        resolved_api_base(Some("http://explicit"), &configured),
        Some(String::from("http://explicit"))
    );
}

#[test]
fn resolved_api_base_falls_through_an_empty_explicit_base() {
    let configured = environment(&[(API_BASE_ENV, "http://env")]);
    assert_eq!(
        resolved_api_base(Some(""), &configured),
        Some(String::from("http://env"))
    );
}

#[test]
fn resolved_api_base_is_absent_without_a_base() {
    assert_eq!(resolved_api_base(None, &environment(&[])), None);
    assert_eq!(
        resolved_api_base(None, &environment(&[(API_BASE_ENV, "")])),
        None
    );
}

#[rstest]
#[case("http://host", "/responses", "http://host/v1/responses")]
#[case("http://host/", "/responses", "http://host/v1/responses")]
#[case("http://host/v1", "/responses", "http://host/v1/responses")]
#[case("http://host/v1/", "/responses", "http://host/v1/responses")]
#[case("http://host/v10", "/videos", "http://host/v10/v1/videos")]
fn versioned_url_appends_v1_unless_the_base_ends_there(
    #[case] base: &str,
    #[case] path: &str,
    #[case] expected: &str,
) {
    assert_eq!(versioned_url(base, path), expected);
}

#[test]
fn responses_url_reads_the_base_from_the_environment() {
    let configured = environment(&[(API_BASE_ENV, "http://env/v1/")]);
    assert_eq!(
        responses::get_complete_url(None, &configured),
        Ok(String::from("http://env/v1/responses"))
    );
}

#[test]
fn responses_url_requires_a_base() {
    assert_eq!(
        responses::get_complete_url(None, &environment(&[])),
        Err(responses::ResponsesConfigError::MissingApiBase)
    );
}

#[test]
fn responses_headers_overwrite_authorization_and_keep_the_rest() {
    let validated = responses::validate_environment(
        headers(&[("Authorization", "Bearer caller"), ("x-extra", "1")]),
        Some(&secret("key")),
        &environment(&[]),
    );
    assert_eq!(
        validated,
        headers(&[("Authorization", "Bearer key"), ("x-extra", "1")])
    );
}

#[test]
fn responses_provider_and_websocket_are_the_hosted_vllm_ones() {
    assert_eq!(
        responses::PROVIDER,
        betterlite_models::identity::provider_id::ProviderId::HostedVllm
    );
    assert!(!responses::SUPPORTS_NATIVE_WEBSOCKET);
}

#[test]
fn image_edit_supported_params_drop_the_three_vllm_omni_refuses() {
    assert_eq!(
        image_edit::get_supported_openai_params(),
        vec![
            "image",
            "prompt",
            "background",
            "model",
            "n",
            "response_format",
            "size",
            "user",
            "extra_headers",
            "extra_query",
            "extra_body",
            "timeout",
        ]
    );
}

#[test]
fn image_edit_url_appends_the_edits_path_under_v1() {
    let configured = environment(&[(API_BASE_ENV, "http://env/v1")]);
    assert_eq!(
        image_edit::get_complete_url(Some("http://explicit/"), &configured),
        Ok(String::from("http://explicit/v1/images/edits"))
    );
}

#[test]
fn image_edit_url_requires_a_base() {
    assert_eq!(
        image_edit::get_complete_url(None, &environment(&[])),
        Err(image_edit::ImageEditConfigError::MissingApiBase)
    );
}

#[test]
fn image_edit_headers_overwrite_authorization_and_keep_the_rest() {
    let validated = image_edit::validate_environment(
        headers(&[("Authorization", "Bearer caller"), ("x-extra", "1")]),
        None,
        &environment(&[(API_KEY_ENV, "env-key")]),
    );
    assert_eq!(
        validated,
        headers(&[("Authorization", "Bearer env-key"), ("x-extra", "1")])
    );
}

// only a base that already ends with the whole path is kept, so a /v1 base gets a second /v1
#[rstest]
#[case(
    Some("http://host/v1/audio/transcriptions/"),
    "http://host/v1/audio/transcriptions"
)]
#[case(Some("http://host/v1"), "http://host/v1/v1/audio/transcriptions")]
#[case(Some("http://host/v2"), "http://host/v2/v1/audio/transcriptions")]
fn transcription_url_keeps_or_appends_the_transcriptions_path(
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    assert_eq!(
        transcription::get_complete_url(api_base).expect("a base is set"),
        expected
    );
}

#[rstest]
#[case(None)]
#[case(Some(""))]
fn transcription_url_requires_a_base(#[case] api_base: Option<&str>) {
    assert!(matches!(
        transcription::get_complete_url(api_base),
        Err(transcription::HostedVllmAudioTranscriptionError::MissingApiBase)
    ));
}

#[test]
fn transcription_request_lets_the_options_model_win_and_adds_no_verbose_json() {
    let options = IndexMap::from([
        (String::from("model"), Json::Text(String::from("other"))),
        (String::from("language"), Json::Text(String::from("en"))),
    ]);
    let data = transcription::transform_audio_transcription_request("whisper", &options)
        .expect("the options are well-formed");
    assert_eq!(data.model, Some(Json::Text(String::from("other"))));
    assert_eq!(
        data.options,
        IndexMap::from([(String::from("language"), Json::Text(String::from("en")))])
    );
}

#[test]
fn transcription_request_defaults_to_the_model_and_refuses_a_file_option() {
    let data = transcription::transform_audio_transcription_request("whisper", &IndexMap::new())
        .expect("no options");
    assert_eq!(data.model, Some(Json::Text(String::from("whisper"))));
    assert!(data.options.is_empty());
    let with_file = IndexMap::from([(String::from("file"), Json::Text(String::from("x")))]);
    assert!(transcription::transform_audio_transcription_request("whisper", &with_file).is_err());
}

fn env_key(name: &str) -> String {
    if name == "HOSTED_VLLM_API_KEY" {
        String::from("env-key")
    } else {
        String::new()
    }
}

fn empty_secret(_name: &str) -> String {
    String::new()
}

#[test]
fn decisions_key_resolution_prefers_the_argument() {
    let config = HostedVllmDecisionsConfig;
    assert_eq!(
        config.resolve_api_key(Some("explicit"), env_key),
        Some(String::from("explicit"))
    );
    assert_eq!(
        config.resolve_api_key(Some("explicit"), empty_secret),
        Some(String::from("explicit"))
    );
}

#[test]
fn decisions_key_resolution_reads_the_environment_after_an_empty_argument() {
    let config = HostedVllmDecisionsConfig;
    assert_eq!(
        config.resolve_api_key(Some(""), env_key),
        Some(String::from("env-key"))
    );
    assert_eq!(
        config.resolve_api_key(None, env_key),
        Some(String::from("env-key"))
    );
}

#[test]
fn decisions_key_resolution_drops_the_placeholder() {
    let config = HostedVllmDecisionsConfig;
    assert_eq!(
        config.resolve_api_key(Some("fake-api-key"), empty_secret),
        None
    );
    assert_eq!(config.resolve_api_key(None, empty_secret), None);
}

#[test]
fn decisions_config_names_the_hosted_vllm_environment_and_needs_no_key() {
    assert_eq!(
        HostedVllmDecisionsConfig::API_KEY_ENV,
        &["HOSTED_VLLM_API_KEY"]
    );
    assert_eq!(
        HostedVllmDecisionsConfig::API_BASE_ENV,
        &["HOSTED_VLLM_API_BASE"]
    );
    assert!(!HostedVllmDecisionsConfig::API_KEY_REQUIRED);
}

#[test]
fn decisions_health_check_is_a_choice_between_yes_and_no() {
    let config = HostedVllmDecisionsConfig;
    assert_eq!(
        config.health_check_questions(),
        vec![ir::DecisionQuestion::Choice(ir::ChoiceQuestion {
            name: Some(String::from("reachable")),
            instructions: Some(DecisionsJson::Text(String::from(
                "Is the service reachable?"
            ))),
            choices: vec![
                ir::ChoiceOption {
                    value: ChoiceValue::Text(String::from("yes")),
                    description: None,
                },
                ir::ChoiceOption {
                    value: ChoiceValue::Text(String::from("no")),
                    description: None,
                },
            ],
            extra: Extra::default(),
        })]
    );
}
