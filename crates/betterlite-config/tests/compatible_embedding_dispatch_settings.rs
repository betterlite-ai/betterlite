#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_config::sections::compatible_embedding_settings::{
    DEFAULT_COMETAPI_API_BASE, EmbeddingFamily, Globals, Settings,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

fn settings(environment: &[(&str, &str)]) -> Settings {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    ConfigLoader { lookup }
        .load("")
        .expect("an empty config loads")
        .sections
        .compatible_embedding_settings
}

fn secret(text: &str) -> SecretString {
    SecretString::from(text.to_owned())
}

const ENVIRONMENT: &[(&str, &str)] = &[
    ("OPENAI_LIKE_API_BASE", "http://openai-like/v1"),
    ("OPENAI_LIKE_API_KEY", "sk-openai-like"),
    ("COMETAPI_API_BASE", "https://comet.example/v1"),
    ("COMETAPI_KEY", "sk-cometapi"),
];

#[rstest]
#[case::all_four(
    ENVIRONMENT,
    Some("http://openai-like/v1"),
    Some("sk-openai-like"),
    Some("https://comet.example/v1"),
    Some("sk-cometapi")
)]
#[case::openai_like_only(&[("OPENAI_LIKE_API_BASE", "http://openai-like/v1"), ("OPENAI_LIKE_API_KEY", "sk-openai-like")], Some("http://openai-like/v1"), Some("sk-openai-like"), None, None)]
#[case::cometapi_only(&[("COMETAPI_API_BASE", "https://comet.example/v1"), ("COMETAPI_KEY", "sk-cometapi")], None, None, Some("https://comet.example/v1"), Some("sk-cometapi"))]
#[case::empty_values_count_as_unset(&[("OPENAI_LIKE_API_BASE", ""), ("OPENAI_LIKE_API_KEY", ""), ("COMETAPI_API_BASE", ""), ("COMETAPI_KEY", "")], None, None, None, None)]
#[case::absent(&[], None, None, None, None)]
fn loads_every_environment_alias(
    #[case] environment: &[(&str, &str)],
    #[case] openai_like_base: Option<&str>,
    #[case] openai_like_key: Option<&str>,
    #[case] cometapi_base: Option<&str>,
    #[case] cometapi_key: Option<&str>,
) {
    let loaded = settings(environment);
    assert_eq!(loaded.openai_like.api_base.as_deref(), openai_like_base);
    assert_eq!(
        loaded
            .openai_like
            .api_key
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        openai_like_key
    );
    assert_eq!(loaded.cometapi_api_base.as_deref(), cometapi_base);
    assert_eq!(
        loaded
            .cometapi_credentials
            .api_key
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        cometapi_key
    );
}

#[rstest]
#[case::openai_like_explicit_wins(
    EmbeddingFamily::OpenAiLike,
    ENVIRONMENT,
    globals(None, None, None, None),
    (Some("http://explicit/v1"), Some("sk-explicit")),
    (Some("http://explicit/v1"), Some("sk-explicit"))
)]
#[case::openai_like_explicit_base_beats_the_global_base(
    EmbeddingFamily::OpenAiLike,
    ENVIRONMENT,
    globals(Some("https://global/v1"), None, None, None),
    (Some("http://explicit/v1"), None),
    (Some("http://explicit/v1"), Some("sk-openai-like"))
)]
#[case::openai_like_api_key_beats_the_openai_like_key(
    EmbeddingFamily::OpenAiLike,
    &[],
    globals(None, Some("sk-api"), Some("sk-like"), None),
    (None, None),
    (None, Some("sk-api"))
)]
#[case::openai_like_global_base_and_key_win(
    EmbeddingFamily::OpenAiLike,
    ENVIRONMENT,
    globals(Some("https://global/v1"), Some("sk-global"), None, None),
    (None, None),
    (Some("https://global/v1"), Some("sk-global"))
)]
#[case::openai_like_global_key(
    EmbeddingFamily::OpenAiLike,
    ENVIRONMENT,
    globals(None, None, Some("sk-like"), None),
    (None, None),
    (Some("http://openai-like/v1"), Some("sk-like"))
)]
#[case::openai_like_environment_takes_over(
    EmbeddingFamily::OpenAiLike,
    ENVIRONMENT,
    globals(None, None, None, None),
    (None, None),
    (Some("http://openai-like/v1"), Some("sk-openai-like"))
)]
#[case::openai_like_nothing_anywhere(
    EmbeddingFamily::OpenAiLike,
    &[],
    globals(None, None, None, None),
    (None, None),
    (None, None)
)]
#[case::openai_like_explicit_empty_key_suppresses(
    EmbeddingFamily::OpenAiLike,
    ENVIRONMENT,
    globals(None, Some("sk-global"), None, None),
    (None, Some("")),
    (Some("http://openai-like/v1"), Some(""))
)]
#[case::openai_like_explicit_empty_base_is_used_as_given(
    EmbeddingFamily::OpenAiLike,
    &[],
    globals(None, None, None, None),
    (Some(""), None),
    (Some(""), None)
)]
#[case::cometapi_explicit_wins(
    EmbeddingFamily::Cometapi,
    ENVIRONMENT,
    globals(None, None, None, None),
    (Some("https://explicit/v1"), Some("sk-explicit")),
    (Some("https://explicit/v1"), Some("sk-explicit"))
)]
#[case::cometapi_global_base_and_cometapi_key(
    EmbeddingFamily::Cometapi,
    ENVIRONMENT,
    globals(
        Some("https://global/v1"),
        Some("sk-api"),
        Some("sk-like"),
        Some("sk-cometapi-global")
    ),
    (None, None),
    (Some("https://global/v1"), Some("sk-cometapi-global"))
)]
#[case::cometapi_environment(
    EmbeddingFamily::Cometapi,
    ENVIRONMENT,
    globals(None, None, None, None),
    (None, None),
    (Some("https://comet.example/v1"), Some("sk-cometapi"))
)]
#[case::cometapi_general_global_key_comes_last(
    EmbeddingFamily::Cometapi,
    ENVIRONMENT,
    globals(None, Some("sk-api"), None, None),
    (None, None),
    (Some("https://comet.example/v1"), Some("sk-cometapi"))
)]
#[case::cometapi_general_global_key_without_environment(
    EmbeddingFamily::Cometapi,
    &[],
    globals(None, Some("sk-api"), None, None),
    (None, None),
    (Some(DEFAULT_COMETAPI_API_BASE), Some("sk-api"))
)]
#[case::cometapi_default_base_and_no_key(
    EmbeddingFamily::Cometapi,
    &[],
    globals(None, None, None, None),
    (None, None),
    (Some(DEFAULT_COMETAPI_API_BASE), None)
)]
#[case::cometapi_explicit_empty_key_suppresses(
    EmbeddingFamily::Cometapi,
    ENVIRONMENT,
    globals(None, Some("sk-api"), None, Some("sk-cometapi-global")),
    (None, Some("")),
    (Some("https://comet.example/v1"), Some(""))
)]
#[case::cometapi_explicit_empty_base_is_used_as_given(
    EmbeddingFamily::Cometapi,
    &[],
    globals(None, None, None, None),
    (Some(""), None),
    (Some(""), None)
)]
fn identity_follows_the_pinned_precedence(
    #[case] family: EmbeddingFamily,
    #[case] environment: &[(&str, &str)],
    #[case] globals: Globals,
    #[case] supplied: (Option<&str>, Option<&str>),
    #[case] expected: (Option<&str>, Option<&str>),
) {
    let (explicit_base, explicit_key) = supplied;
    let (expected_base, expected_key) = expected;
    let supplied_key = explicit_key.map(secret);
    let identity =
        settings(environment).identity(family, &globals, explicit_base, supplied_key.as_ref());
    assert_eq!(identity.api_base.as_deref(), expected_base);
    assert_eq!(
        identity
            .api_key
            .as_ref()
            .map(|secret| secret.expose_secret().to_owned())
            .as_deref(),
        expected_key
    );
}

fn globals(
    api_base: Option<&str>,
    api_key: Option<&str>,
    openai_like_key: Option<&str>,
    cometapi_key: Option<&str>,
) -> Globals {
    Globals {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(secret),
        openai_like_key: openai_like_key.map(secret),
        cometapi_key: cometapi_key.map(secret),
    }
}
