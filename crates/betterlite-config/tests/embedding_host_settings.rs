use std::collections::BTreeMap;

use betterlite_config::loader::{ConfigError, ConfigLoader};
use rstest::rstest;
use secrecy::ExposeSecret as _;

type Loaded = (Option<String>, Option<String>, Option<String>);

fn load(environment: &[(&str, &str)]) -> Result<Loaded, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let settings = ConfigLoader { lookup }
        .load("")?
        .sections
        .embedding_host_settings;
    Ok((
        settings.jina_ai_api_base,
        settings
            .jina_ai_api_key
            .map(|secret| secret.expose_secret().to_owned()),
        settings
            .infinity_api_key
            .map(|secret| secret.expose_secret().to_owned()),
    ))
}

#[rstest]
#[case::base(&[("JINA_AI_API_BASE", "https://jina.test/v1")], Some("https://jina.test/v1"), None)]
#[case::empty_base(&[("JINA_AI_API_BASE", "")], None, None)]
#[case::api_key(&[("JINA_AI_API_KEY", "first")], None, Some("first"))]
#[case::short_key(&[("JINA_API_KEY", "second")], None, Some("second"))]
#[case::token(&[("JINA_AI_TOKEN", "third")], None, Some("third"))]
#[case::precedence(&[("JINA_AI_API_KEY", "first"), ("JINA_API_KEY", "second"), ("JINA_AI_TOKEN", "third")], None, Some("first"))]
#[case::second_over_token(&[("JINA_API_KEY", "second"), ("JINA_AI_TOKEN", "third")], None, Some("second"))]
#[case::empty_falls_through(&[("JINA_AI_API_KEY", ""), ("JINA_API_KEY", ""), ("JINA_AI_TOKEN", "third")], None, Some("third"))]
#[case::empty_token(&[("JINA_AI_TOKEN", "")], None, None)]
#[case::unset(&[], None, None)]
fn jina_environment_is_resolved(
    #[case] environment: &[(&str, &str)],
    #[case] base: Option<&str>,
    #[case] key: Option<&str>,
) {
    let (loaded_base, loaded_key, infinity_key) = load(environment).unwrap();
    assert_eq!(loaded_base.as_deref(), base);
    assert_eq!(loaded_key.as_deref(), key);
    assert_eq!(infinity_key, None);
}

#[rstest]
#[case::set(&[("INFINITY_API_KEY", "secret")], Some("secret"))]
#[case::empty(&[("INFINITY_API_KEY", "")], None)]
#[case::unset(&[], None)]
#[case::jina_key_is_not_infinity(&[("JINA_AI_API_KEY", "jina")], None)]
fn infinity_key_is_read_alone(#[case] environment: &[(&str, &str)], #[case] key: Option<&str>) {
    let (base, _jina_key, infinity_key) = load(environment).unwrap();
    assert_eq!(infinity_key.as_deref(), key);
    assert_eq!(base, None);
}

#[rstest]
#[case::global_only(&[], None, Some("global"))]
#[case::environment_and_global(&[("INFINITY_API_KEY", "env")], Some("env"), Some("global"))]
fn infinity_rerank_global_key(
    #[case] environment: &[(&str, &str)],
    #[case] embedding_key: Option<&str>,
    #[case] rerank_global_key: Option<&str>,
) {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let sections = ConfigLoader { lookup }
        .load("litellm_settings:\n  infinity_key: global\n")
        .unwrap()
        .sections;
    let embedding = sections
        .embedding_host_settings
        .infinity_api_key
        .map(|secret| secret.expose_secret().to_owned());
    let rerank = sections
        .jina_infinity_rerank
        .infinity_key
        .map(|secret| secret.expose_secret().to_owned());
    assert_eq!(embedding.as_deref(), embedding_key);
    assert_eq!(rerank.as_deref(), rerank_global_key);
}
