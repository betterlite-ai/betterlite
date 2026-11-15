#![cfg(test)]

use betterlite_config::sections::vertex_credential_sources::VertexSettings;
use betterlite_config::sections::vertex_credential_sources::vertex_settings::AdcEnvironment;
use rstest::rstest;

use crate::credential_sources::getters::VertexParams;
use crate::partner_count::error::PartnerCountError;
use crate::partner_count::options::PartnerCountOptions;
use crate::partner_count::plan::{
    DEFAULT_COUNT_TOKENS_LOCATION, PartnerPublisher, build_count_tokens_endpoint,
    count_tokens_location, publisher_for_model, strip_version_suffix,
};

#[rstest]
#[case::claude("claude-3-5-sonnet-20241022", PartnerPublisher::Anthropic)]
#[case::mistral("mistral-large-2411", PartnerPublisher::MistralAi)]
#[case::codestral("codestral-2501", PartnerPublisher::MistralAi)]
#[case::llama("llama-3.3-70b-instruct-maas", PartnerPublisher::Meta)]
#[case::meta_prefix("meta/llama-4-scout", PartnerPublisher::Meta)]
#[case::claude_wins_over_mistral("claude-mistral", PartnerPublisher::Anthropic)]
#[case::mistral_wins_over_llama("mistral-large-llama", PartnerPublisher::MistralAi)]
fn publishers_come_from_the_model_substring(
    #[case] model: &str,
    #[case] publisher: PartnerPublisher,
) {
    assert_eq!(publisher_for_model(model).unwrap(), publisher);
}

#[test]
fn an_unknown_partner_model_names_itself() {
    let error = publisher_for_model("gemini-2.5-pro").unwrap_err();

    assert_eq!(error.to_string(), "Unknown partner model: gemini-2.5-pro");
    assert_eq!(error.status_code(), None);
}

#[test]
fn the_publisher_match_is_case_sensitive() {
    let error = publisher_for_model("Claude-3-5-sonnet").unwrap_err();

    assert!(matches!(error, PartnerCountError::UnknownPublisher { .. }));
}

#[rstest]
#[case::default("claude-sonnet-4-6@default", "claude-sonnet-4-6")]
#[case::dated("claude-sonnet-4-6@20251001", "claude-sonnet-4-6")]
#[case::first_at("claude-sonnet-4-6@a@b", "claude-sonnet-4-6")]
#[case::leading_at("@default", "")]
#[case::plain("claude-sonnet-4-6", "claude-sonnet-4-6")]
fn the_version_suffix_is_stripped_at_the_first_at(#[case] model: &str, #[case] stripped: &str) {
    assert_eq!(strip_version_suffix(model), stripped);
}

#[rstest]
#[case::claude(
    "claude-3-5-sonnet",
    "proj",
    "us-east5",
    None,
    "https://us-east5-aiplatform.googleapis.com/v1/projects/proj/locations/us-east5/publishers/anthropic/models/count-tokens:rawPredict"
)]
#[case::global(
    "claude-3-5-sonnet",
    "proj",
    "global",
    None,
    "https://aiplatform.googleapis.com/v1/projects/proj/locations/global/publishers/anthropic/models/count-tokens:rawPredict"
)]
#[case::multi_region(
    "mistral-large",
    "proj",
    "us",
    None,
    "https://aiplatform.us.rep.googleapis.com/v1/projects/proj/locations/us/publishers/mistralai/models/count-tokens:rawPredict"
)]
#[case::meta(
    "meta/llama-3.1-405b",
    "proj-2",
    "europe-west1",
    None,
    "https://europe-west1-aiplatform.googleapis.com/v1/projects/proj-2/locations/europe-west1/publishers/meta/models/count-tokens:rawPredict"
)]
#[case::custom_base(
    "claude-3-5-sonnet",
    "proj",
    "us-east5",
    Some("https://proxy.example/v1"),
    "https://proxy.example/v1/v1/projects/proj/locations/us-east5/publishers/anthropic/models/count-tokens:rawPredict"
)]
#[case::empty_base_falls_back(
    "claude-3-5-sonnet",
    "proj",
    "us-east5",
    Some(""),
    "https://us-east5-aiplatform.googleapis.com/v1/projects/proj/locations/us-east5/publishers/anthropic/models/count-tokens:rawPredict"
)]
fn the_endpoint_names_the_publisher_and_location(
    #[case] model: &str,
    #[case] project: &str,
    #[case] location: &str,
    #[case] api_base: Option<&str>,
    #[case] endpoint: &str,
) {
    assert_eq!(
        build_count_tokens_endpoint(model, project, location, api_base).unwrap(),
        endpoint
    );
}

#[test]
fn an_unknown_publisher_has_no_endpoint() {
    let error =
        build_count_tokens_endpoint("gemini-2.5-pro", "proj", "us-east5", None).unwrap_err();

    assert!(matches!(error, PartnerCountError::UnknownPublisher { .. }));
}

#[test]
fn an_invalid_location_cannot_choose_the_host() {
    let error = build_count_tokens_endpoint("claude-3-5-sonnet", "proj", "evil.example/", None)
        .unwrap_err();

    assert!(matches!(error, PartnerCountError::VertexUrl(_)));
}

#[rstest]
#[case::count_tokens_first(
    Some("asia-southeast1"),
    Some("europe-west1"),
    Some("us-central1"),
    "asia-southeast1"
)]
#[case::empty_override_falls_through(
    Some(""),
    Some("europe-west1"),
    Some("us-central1"),
    "europe-west1"
)]
#[case::deployment_before_settings(None, Some("europe-west1"), Some("us-central1"), "europe-west1")]
#[case::settings_next(None, None, Some("us-central1"), "us-central1")]
#[case::default_for_every_model(None, None, None, DEFAULT_COUNT_TOKENS_LOCATION)]
fn the_count_tokens_location_is_chosen_in_order(
    #[case] configured_location: Option<&str>,
    #[case] deployment_location: Option<&str>,
    #[case] settings_location: Option<&str>,
    #[case] expected: &str,
) {
    let options = PartnerCountOptions {
        vertex_count_tokens_location: configured_location.map(str::to_owned),
    };
    let settings = VertexSettings {
        project: None,
        location: settings_location.map(str::to_owned),
        credentials: None,
        adc: AdcEnvironment::default(),
    };
    let params = VertexParams {
        project: None,
        ai_project: None,
        location: deployment_location,
        ai_location: None,
        credentials: None,
        ai_credentials: None,
    };

    assert_eq!(count_tokens_location(&options, params, &settings), expected);
}

#[test]
fn the_default_location_is_us_east5() {
    assert_eq!(DEFAULT_COUNT_TOKENS_LOCATION, "us-east5");
}
