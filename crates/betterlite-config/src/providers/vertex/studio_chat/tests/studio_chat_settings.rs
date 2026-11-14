use betterlite_vertex::studio_chat::settings::StudioSettings;
use serde::Deserialize;

use crate::loader::{ConfigError, parse_view};

const CONFIGURED: &str = "model_list:\n  - litellm_params:\n      model: gemini/gemini-3-pro-preview\n      temperature: 0\n      max_output_tokens: 256\n      top_p: 0.95\n      top_k: 40\n      response_mime_type: application/json\n      response_schema:\n        type: object\n        properties:\n          name: {type: string}\n      candidate_count: 2\n      stop_sequences: [END]\n";

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: StudioSettings,
}

// the loader's own view parses the class defaults until the section registry carries an entry for them
fn load(source: &str) -> Result<StudioSettings, ConfigError> {
    let (raw, _unknown) = parse_view::<Raw>(source)?;
    Ok(raw
        .model_list
        .into_iter()
        .next()
        .map(|deployment| deployment.litellm_params)
        .unwrap_or_default())
}

#[test]
fn a_configured_response_schema_reaches_the_studio_options() {
    let options = load(CONFIGURED)
        .expect("the loader's view parses the keys")
        .get_config()
        .expect("every configured value is finite");
    let config = serde_json::to_value(options).expect("the options encode as JSON");

    let schema = config
        .get("response_schema")
        .expect("the configured schema reaches the options");
    assert_eq!(
        schema.get("type"),
        Some(&serde_json::to_value("object").unwrap())
    );
    assert_eq!(
        schema.pointer("/properties/name/type"),
        Some(&serde_json::to_value("string").unwrap())
    );
    assert_eq!(
        config.get("response_mime_type"),
        Some(&serde_json::to_value("application/json").unwrap())
    );
    assert_eq!(config.as_object().unwrap().len(), 8, "{config}");
}

#[test]
fn every_configured_value_keeps_its_type() {
    let options = load(CONFIGURED)
        .expect("the loader's view parses the keys")
        .get_config()
        .expect("every configured value is finite");
    let config = serde_json::to_value(options).expect("the options encode as JSON");

    assert_eq!(
        config.get("temperature"),
        Some(&serde_json::to_value(0.0_f64).unwrap())
    );
    assert_eq!(
        config.get("max_output_tokens"),
        Some(&serde_json::to_value(256_i64).unwrap())
    );
    assert_eq!(
        config.get("top_p"),
        Some(&serde_json::to_value(0.95_f64).unwrap())
    );
    assert_eq!(
        config.get("top_k"),
        Some(&serde_json::to_value(40_i64).unwrap())
    );
    assert_eq!(
        config.get("candidate_count"),
        Some(&serde_json::to_value(2_i64).unwrap())
    );
    assert_eq!(
        config.get("stop_sequences"),
        Some(&serde_json::to_value(["END"]).unwrap())
    );
}

#[test]
fn the_eight_class_defaults_start_absent() {
    let settings = StudioSettings::default();
    let config = settings.get_config().expect("no value to reject");

    assert_eq!(settings.temperature, None);
    assert_eq!(settings.max_output_tokens, None);
    assert_eq!(settings.top_p, None);
    assert_eq!(settings.top_k, None);
    assert_eq!(settings.response_mime_type, None);
    assert_eq!(settings.response_schema, None);
    assert_eq!(settings.candidate_count, None);
    assert_eq!(settings.stop_sequences, None);
    assert!(config.is_empty(), "{config:?}");
}

#[test]
fn a_null_response_schema_takes_the_default() {
    let settings = load("model_list:\n  - litellm_params:\n      response_schema: null\n")
        .expect("a null key takes the default");

    assert_eq!(settings.response_schema, None);
    assert!(settings.get_config().expect("nothing to reject").is_empty());
}

#[test]
fn a_response_schema_that_is_not_an_object_names_itself() {
    let error = load("model_list:\n  - litellm_params:\n      response_schema: [type]\n")
        .expect_err("a list is not a schema object");

    assert!(error.to_string().contains("response_schema"), "{error}");
}

#[test]
fn a_non_numeric_generation_value_names_itself() {
    let error = load("model_list:\n  - litellm_params:\n      top_p: warm\n")
        .expect_err("a string is not a top_p");

    assert!(error.to_string().contains("top_p"), "{error}");
}

#[test]
fn a_non_finite_generation_value_names_itself() {
    let settings = StudioSettings {
        temperature: None,
        max_output_tokens: None,
        top_p: Some(f64::INFINITY),
        top_k: None,
        response_mime_type: None,
        response_schema: None,
        candidate_count: None,
        stop_sequences: None,
    };

    let error = settings
        .get_config()
        .expect_err("infinity has no JSON number");

    assert!(error.to_string().contains("top_p"), "{error}");
}
