#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

use super::{DEFAULT_EMBEDDING_ENCODING_FORMAT_ENV, EXPERIMENTAL_OPENAI_BASE_LLM_HTTP_HANDLER_ENV};

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

#[rstest]
#[case::loads_experimental_openai_base_llm_http_handler(0)]
#[case::loads_enable_preview_features(1)]
#[case::loads_default_embedding_encoding_format(2)]
#[case::rejects_unrelated_key(3)]
fn openai_provider_config_cases(#[case] case: u8) {
    match case {
        0 => {
            let name = EXPERIMENTAL_OPENAI_BASE_LLM_HTTP_HANDLER_ENV;
            assert!(!load("", &[]).unwrap().experimental_base_llm_http_handler);
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
                    loaded.experimental_base_llm_http_handler, expected,
                    "EXPERIMENTAL_OPENAI_BASE_LLM_HTTP_HANDLER={given}"
                );
                assert!(!loaded.enable_preview_features);
                assert!(loaded.default_embedding_encoding_format.is_none());
            }
        }
        1 => {
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
                load("litellm_settings:\n  enable_preview_features: sometimes\n", &[]),
                Err(ConfigError::InvalidScalar { .. })
            ));
            assert!(matches!(
                load("litellm_settings:\n  enable_preview_features: 1\n", &[]),
                Err(ConfigError::InvalidScalar { .. })
            ));
        }
        2 => {
            assert!(load("", &[]).unwrap().default_embedding_encoding_format.is_none());
            let loaded = load("", &[(DEFAULT_EMBEDDING_ENCODING_FORMAT_ENV, "base64")]).unwrap();
            assert_eq!(
                loaded.default_embedding_encoding_format.as_deref(),
                Some("base64")
            );
            let none = load("", &[(DEFAULT_EMBEDDING_ENCODING_FORMAT_ENV, "")]).unwrap();
            assert!(none.default_embedding_encoding_format.is_none());
            let sentinel = load("", &[(DEFAULT_EMBEDDING_ENCODING_FORMAT_ENV, "none")]).unwrap();
            assert_eq!(
                sentinel.default_embedding_encoding_format.as_deref(),
                Some("none")
            );
        }
        _ => {
            let source = "litellm_settings:\n  openai_provider_unrelated_key: 1\n";
            assert!(matches!(
                load(source, &[]),
                Err(ConfigError::Unsupported { .. })
            ));
        }
    }
}
