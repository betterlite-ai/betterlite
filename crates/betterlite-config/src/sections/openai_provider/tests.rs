#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{EMBEDDING_FORMAT_ENV, EXPERIMENTAL_HANDLER_ENV};

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .openai_provider)
}

// Only a true spelling turns the handler on; any other value leaves it off.
fn check_experimental_flag() {
    let name = EXPERIMENTAL_HANDLER_ENV;
    assert!(!load("", &[]).unwrap().experimental_handler);
    for (given, expected) in [
        ("true", true),
        ("TRUE", true),
        (" true ", true),
        ("false", false),
        ("False", false),
        ("0", false),
        ("yes", false),
        ("", false),
    ] {
        let loaded = load("", &[(name, given)]).unwrap();
        assert_eq!(
            loaded.experimental_handler, expected,
            "EXPERIMENTAL_OPENAI_BASE_LLM_HTTP_HANDLER={given}"
        );
        assert!(!loaded.enable_preview_features);
        assert!(loaded.embedding_format.is_none());
    }
}

// The preview flag comes from litellm_settings, resolves an environment reference, and refuses a non-boolean.
fn check_preview_flag() {
    assert!(!load("", &[]).unwrap().enable_preview_features);
    assert!(
        !load("litellm_settings: {}\n", &[])
            .unwrap()
            .enable_preview_features
    );
    assert!(
        load("litellm_settings:\n  enable_preview_features: true\n", &[])
            .unwrap()
            .enable_preview_features
    );
    assert!(
        !load("litellm_settings:\n  enable_preview_features: false\n", &[])
            .unwrap()
            .enable_preview_features
    );
    let resolved = load(
        "litellm_settings:\n  enable_preview_features: os.environ/PREVIEW\n",
        &[("PREVIEW", "true")],
    )
    .unwrap();
    assert!(resolved.enable_preview_features);
    assert!(matches!(
        load(
            "litellm_settings:\n  enable_preview_features: sometimes\n",
            &[]
        ),
        Err(ConfigError::InvalidScalar { .. })
    ));
    assert!(matches!(
        load("litellm_settings:\n  enable_preview_features: 1\n", &[]),
        Err(ConfigError::InvalidScalar { .. })
    ));
}

// The embedding format reads the environment variable, and an empty value counts as unset.
fn check_embedding_format() {
    assert!(load("", &[]).unwrap().embedding_format.is_none());
    let loaded = load("", &[(EMBEDDING_FORMAT_ENV, "base64")]).unwrap();
    assert_eq!(loaded.embedding_format.as_deref(), Some("base64"));
    let none = load("", &[(EMBEDDING_FORMAT_ENV, "")]).unwrap();
    assert!(none.embedding_format.is_none());
    let sentinel = load("", &[(EMBEDDING_FORMAT_ENV, "none")]).unwrap();
    assert_eq!(sentinel.embedding_format.as_deref(), Some("none"));
}

// A key the section does not read stays unsupported.
fn check_unrelated_key_refused() {
    let source = "litellm_settings:\n  openai_provider_unrelated_key: 1\n";
    assert!(matches!(
        load(source, &[]),
        Err(ConfigError::Unsupported { .. })
    ));
}

#[rstest]
#[case::loads_experimental_openai_base_llm_http_handler(0)]
#[case::loads_enable_preview_features(1)]
#[case::loads_default_embedding_encoding_format(2)]
#[case::rejects_unrelated_key(3)]
fn openai_provider_config_cases(#[case] case: u8) {
    match case {
        0 => check_experimental_flag(),
        1 => check_preview_flag(),
        2 => check_embedding_format(),
        _ => check_unrelated_key_refused(),
    }
}
