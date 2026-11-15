use betterlite_openai::base_helpers::provider_config;
use betterlite_vertex::partner_openai::settings::PartnerSettings;
use serde::Deserialize;

use crate::loader::{ConfigError, parse_view};

const CONFIGURED: &str = "model_list:\n  - litellm_params:\n      model: vertex_ai/jamba-1.5-large@001\n      max_tokens: 64\n";
const UNCONFIGURED: &str =
    "model_list:\n  - litellm_params:\n      model: vertex_ai/jamba-1.5-large@001\n";

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: PartnerSettings,
}

// the loader's own view parses the key until the section registry carries an entry for it
fn load(source: &str) -> Result<PartnerSettings, ConfigError> {
    let (raw, _unknown) = parse_view::<Raw>(source)?;
    Ok(raw
        .model_list
        .into_iter()
        .next()
        .map(|deployment| deployment.litellm_params)
        .unwrap_or_default())
}

#[test]
fn a_configured_max_tokens_reaches_the_partner_snapshot() {
    let settings = load(CONFIGURED).expect("the loader's view parses max_tokens");

    assert_eq!(settings.max_tokens, Some(64));
    let snapshot = provider_config(&settings).expect("the settings serialize");
    assert_eq!(snapshot.get("max_tokens"), Some(&serde_json::json!(64)));
    assert_eq!(snapshot.len(), 1);
}

#[test]
fn an_unset_max_tokens_keeps_the_class_default() {
    let settings = load(UNCONFIGURED).expect("a config without the key loads");

    assert_eq!(settings, PartnerSettings::default());
    assert_eq!(settings.max_tokens, None);
    let snapshot = provider_config(&settings).expect("the settings serialize");
    assert!(snapshot.is_empty(), "{snapshot:?}");
}

#[test]
fn a_null_max_tokens_keeps_the_class_default() {
    let settings = load("model_list:\n  - litellm_params:\n      max_tokens: null\n")
        .expect("a null key takes the default");

    assert_eq!(settings.max_tokens, None);
}

#[test]
fn a_non_integer_max_tokens_names_the_field() {
    let error = load("model_list:\n  - litellm_params:\n      max_tokens: many\n")
        .expect_err("a word is not a token count");

    assert!(error.to_string().contains("max_tokens"), "{error}");
}
