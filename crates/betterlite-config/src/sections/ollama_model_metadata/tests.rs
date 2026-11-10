#![cfg(test)]

use crate::loader::{ConfigError, ConfigLoader};
use rstest::rstest;
use secrecy::ExposeSecret as _;
use std::collections::BTreeMap;

fn load(environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load("")?
        .sections
        .ollama_model_metadata)
}

#[rstest]
#[case::loads_ollama_api_key(false)]
#[case::loads_ollama_api_base(true)]
fn ollama_model_metadata_config_cases(#[case] base: bool) {
    let default = load(&[]).unwrap();
    assert!(default.api_key.is_none());
    assert!(default.api_base.is_none());
    if base {
        let value = load(&[("OLLAMA_API_BASE", "http://host:11434")]).unwrap();
        assert_eq!(value.api_base.as_deref(), Some("http://host:11434"));
        let empty = load(&[("OLLAMA_API_BASE", "")]).unwrap();
        assert!(empty.api_base.is_none());
        let untrimmed = load(&[("OLLAMA_API_BASE", " http://host:1 ")])
            .unwrap()
            .api_base;
        assert_eq!(untrimmed.as_deref(), Some(" http://host:1 "));
    } else {
        let key = load(&[("OLLAMA_API_KEY", "marker-key")]).unwrap();
        assert_eq!(key.api_key.expect("API key").expose_secret(), "marker-key");
        let empty = load(&[("OLLAMA_API_KEY", "")]).unwrap();
        assert!(empty.api_key.is_none());
    }
}

#[rstest]
#[case::unrelated_settings_key("litellm_settings:\n  ollama_model_metadata_unrelated: 1\n")]
#[case::unrelated_deployment_key(
    "model_list:\n- model_name: a\n  litellm_params:\n    ollama_model_metadata_unrelated: 1\n"
)]
fn rejects_unrelated_ollama_model_metadata_key(#[case] source: &str) {
    assert!(matches!(
        ConfigLoader {
            lookup: BTreeMap::<String, String>::new(),
        }
        .load(source),
        Err(ConfigError::Unsupported { .. })
    ));
}
