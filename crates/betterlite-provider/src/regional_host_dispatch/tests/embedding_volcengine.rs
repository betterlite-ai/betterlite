use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::regional_host_dispatch::error::DispatchError;

use super::embedding::call_for;
use super::support::{EmbeddingFixture, headers, secret};

const VOLCENGINE_MESSAGE: &str = "Missing API key for Volcengine. Set ARK_API_KEY or VOLCENGINE_API_KEY environment variable or pass api_key parameter.";

// the VOLCENGINE_API_KEY env reaches the arm through get_llm_provider, ahead of litellm.api_key and ARK_API_KEY
#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-global"),
    Some("sk-ark"),
    Some("sk-volcengine"),
    Some("sk-request")
)]
#[case::global_key(None, Some("sk-global"), Some("sk-ark"), None, Some("sk-global"))]
#[case::volcengine_env(None, None, None, Some("sk-volcengine"), Some("sk-volcengine"))]
#[case::volcengine_env_beats_the_global_key(
    None,
    Some("sk-global"),
    None,
    Some("sk-volcengine"),
    Some("sk-volcengine")
)]
#[case::volcengine_env_beats_the_ark_key(
    None,
    None,
    Some("sk-ark"),
    Some("sk-volcengine"),
    Some("sk-volcengine")
)]
#[case::ark_env(None, None, Some("sk-ark"), None, Some("sk-ark"))]
fn volcengine_key_chain(
    #[case] request_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] ark_key: Option<&str>,
    #[case] volcengine_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = EmbeddingFixture::baseline();
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.environment.ark_api_key = ark_key.map(secret);
    fixture.settings.environment.volcengine_api_key = volcengine_key.map(secret);
    let call = call_for(ProviderId::Volcengine, &fixture).expect("the arm answers");
    assert_eq!(
        call.api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        expected.map(str::to_owned)
    );
}

#[test]
fn volcengine_without_a_key_names_both_environment_variables() {
    let fixture = EmbeddingFixture::baseline();
    let outcome = call_for(ProviderId::Volcengine, &fixture);
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey {
            message: VOLCENGINE_MESSAGE
        })
    ));
}

#[test]
fn volcengine_passes_extra_headers_and_an_empty_litellm_params() {
    let mut fixture = EmbeddingFixture::baseline();
    fixture.api_key = Some(secret("sk-request"));
    fixture.extra_headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.api_base = Some(String::from("https://ark.example/v1"));
    fixture.optional_params =
        serde_json::from_str(r#"{"dimensions":2}"#).expect("the fixture parses");
    let call = call_for(ProviderId::Volcengine, &fixture).expect("the arm answers");
    assert_eq!(call.headers, Some(headers(&[("x-scope", "tenant")])));
    assert_eq!(call.api_base, fixture.api_base);
    assert!(call.litellm_params.is_empty());
    assert_eq!(call.optional_params, fixture.optional_params);
}

#[test]
fn volcengine_without_extra_headers_passes_an_empty_map() {
    let mut fixture = EmbeddingFixture::baseline();
    fixture.api_key = Some(secret("sk-request"));
    let call = call_for(ProviderId::Volcengine, &fixture).expect("the arm answers");
    assert_eq!(call.headers, Some(headers(&[])));
}
