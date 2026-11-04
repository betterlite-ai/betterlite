use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use rstest::rstest;
use secrecy::ExposeSecret as _;

fn credentials(environment: &[(&str, &str)]) -> Option<String> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("empty config loads")
        .sections
        .voyage_credentials
        .api_key
        .map(|secret| secret.expose_secret().to_owned())
}

#[rstest]
#[case::primary_alias(&[("VOYAGE_API_KEY", "first")], Some("first"))]
#[case::second_alias(&[("VOYAGE_AI_API_KEY", "second")], Some("second"))]
#[case::third_alias(&[("VOYAGE_AI_TOKEN", "third")], Some("third"))]
#[case::precedence(&[("VOYAGE_API_KEY", "first"), ("VOYAGE_AI_API_KEY", "second"), ("VOYAGE_AI_TOKEN", "third")], Some("first"))]
#[case::empty_falls_through(&[("VOYAGE_API_KEY", ""), ("VOYAGE_AI_API_KEY", "second")], Some("second"))]
#[case::missing(&[], None)]
fn voyage_credential_aliases(#[case] env: &[(&str, &str)], #[case] expected: Option<&str>) {
    assert_eq!(credentials(env).as_deref(), expected);
}
