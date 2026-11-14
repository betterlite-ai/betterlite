use betterlite_models::decisions::Json;
use betterlite_vertex::gemini_options::defaults::GenerationDefaults;
use serde::Deserialize;

use crate::loader::{ConfigError, parse_view};

const CONFIGURED: &str = "litellm_settings:\n  vertex_ai_safety_settings:\n    category: HARM_CATEGORY_HATE_SPEECH\n    threshold: BLOCK_LOW_AND_ABOVE\nmodel_list:\n  - litellm_params:\n      model: vertex_ai/gemini-2.5-pro\n      temperature: 0\n      max_output_tokens: 256\n      top_p: 0.95\n      top_k: 40\n      response_mime_type: application/json\n      candidate_count: 2\n      stop_sequences: [END]\n      frequency_penalty: 0.1\n      presence_penalty: 0.2\n      seed: 7\n  - litellm_params:\n      model: vertex_ai/gemini-3-pro-preview\n";

type Settings = (Vec<GenerationDefaults>, Option<Json>);

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
    #[serde(default)]
    litellm_settings: RawLiteLlm,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: GenerationDefaults,
}

#[derive(Debug, Default, Deserialize)]
struct RawLiteLlm {
    vertex_ai_safety_settings: Option<Json>,
}

// the loader's own view parses the two keys until the section registry carries an entry for them
fn load(source: &str) -> Result<Settings, ConfigError> {
    let (raw, _unknown) = parse_view::<Raw>(source)?;
    Ok((
        raw.model_list
            .into_iter()
            .map(|deployment| deployment.litellm_params)
            .collect(),
        raw.litellm_settings.vertex_ai_safety_settings,
    ))
}

fn configured() -> GenerationDefaults {
    GenerationDefaults {
        temperature: Some(0.0),
        max_output_tokens: Some(256),
        top_p: Some(0.95),
        top_k: Some(40),
        response_mime_type: Some("application/json".to_owned()),
        candidate_count: Some(2),
        stop_sequences: Some(vec!["END".to_owned()]),
        frequency_penalty: Some(0.1),
        presence_penalty: Some(0.2),
        seed: Some(7),
    }
}

#[test]
fn loads_generation_defaults_and_safety_settings() {
    let (deployments, safety) = load(CONFIGURED).expect("the loader's view parses the keys");

    assert_eq!(deployments, [configured(), GenerationDefaults::default()]);
    assert_eq!(
        safety,
        Some(Json::Object(
            [
                (
                    "category".to_owned(),
                    Json::Text("HARM_CATEGORY_HATE_SPEECH".to_owned())
                ),
                (
                    "threshold".to_owned(),
                    Json::Text("BLOCK_LOW_AND_ABOVE".to_owned())
                ),
            ]
            .into_iter()
            .collect()
        ))
    );
}

#[test]
fn an_unset_safety_override_stays_absent() {
    let (deployments, safety) = load("model_list: []\n").expect("an empty config loads");

    assert_eq!(safety, None);
    assert!(deployments.is_empty());
}

#[test]
fn a_null_safety_override_is_absent() {
    let (_, safety) = load("litellm_settings:\n  vertex_ai_safety_settings: null\n")
        .expect("a null key takes the default");

    assert_eq!(safety, None);
}

#[test]
fn a_non_numeric_generation_value_names_itself() {
    let error = load("model_list:\n  - litellm_params:\n      temperature: warm\n")
        .expect_err("a string is not a temperature");

    assert!(error.to_string().contains("temperature"), "{error}");
}
