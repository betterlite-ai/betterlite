use std::collections::BTreeMap;

use betterlite_config::loader::{ConfigError, ConfigLoader, EnvironmentLookup};
use betterlite_openai::voyage_multimodal::{VoyageMultimodalError, credentials as multimodal_key};
use betterlite_openai::voyage_rerank::{VoyageRerankError, api_key as rerank_key};
use rstest::rstest;
use secrecy::ExposeSecret as _;

fn environment(environment: &[(&str, &str)]) -> BTreeMap<String, String> {
    environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn credentials(environment: &[(&str, &str)]) -> Result<Option<String>, ConfigError> {
    Ok(ConfigLoader {
        lookup: self::environment(environment),
    }
    .load("")?
    .sections
    .voyage_credentials
    .api_key
    .map(|secret| secret.expose_secret().to_owned()))
}

#[rstest]
#[case::primary_alias(&[("VOYAGE_API_KEY", "first")], Some("first"))]
#[case::second_alias(&[("VOYAGE_AI_API_KEY", "second")], Some("second"))]
#[case::third_alias(&[("VOYAGE_AI_TOKEN", "third")], Some("third"))]
#[case::precedence(&[("VOYAGE_API_KEY", "first"), ("VOYAGE_AI_API_KEY", "second"), ("VOYAGE_AI_TOKEN", "third")], Some("first"))]
#[case::empty_falls_through(&[("VOYAGE_API_KEY", ""), ("VOYAGE_AI_API_KEY", "second")], Some("second"))]
#[case::missing(&[], None)]
#[case::second_precedence(&[("VOYAGE_AI_API_KEY", "second"), ("VOYAGE_AI_TOKEN", "third")], Some("second"))]
#[case::empty_second(&[("VOYAGE_API_KEY", ""), ("VOYAGE_AI_API_KEY", ""), ("VOYAGE_AI_TOKEN", "third")], Some("third"))]
#[case::empty_third(&[("VOYAGE_AI_TOKEN", "")], None)]
fn voyage_credential_aliases(#[case] env: &[(&str, &str)], #[case] expected: Option<&str>) {
    assert_eq!(
        credentials(env).expect("empty config loads").as_deref(),
        expected
    );
}

#[test]
fn rerank_ignores_token_alias() {
    let token_only = environment(&[("VOYAGE_AI_TOKEN", "third")]);
    let resolved = rerank_key(None, |name| token_only.lookup(name));
    assert!(matches!(resolved, Err(VoyageRerankError::MissingApiKey)));
    let with_second = environment(&[
        ("VOYAGE_AI_TOKEN", "third"),
        ("VOYAGE_AI_API_KEY", "second"),
    ]);
    let resolved = rerank_key(None, |name| with_second.lookup(name)).expect("second alias");
    assert_eq!(resolved, "second");
}

#[test]
fn multimodal_requires_truthy_key() {
    let all_empty = environment(&[
        ("VOYAGE_API_KEY", ""),
        ("VOYAGE_AI_API_KEY", ""),
        ("VOYAGE_AI_TOKEN", ""),
    ]);
    let resolved = multimodal_key(None, |name| all_empty.lookup(name));
    assert!(matches!(
        resolved,
        Err(VoyageMultimodalError::MissingApiKey)
    ));
    let token_only = environment(&[("VOYAGE_AI_TOKEN", "third")]);
    let resolved = multimodal_key(None, |name| token_only.lookup(name)).expect("token alias");
    assert_eq!(resolved, "third");
}
