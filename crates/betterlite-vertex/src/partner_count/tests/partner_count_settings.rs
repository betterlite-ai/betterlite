use crate::partner_count::options::PartnerCountOptions;
use serde::Deserialize;

use betterlite_config::loader::{ConfigError, parse_view};

const CONFIGURED: &str = "model_list:\n  - litellm_params:\n      model: vertex_ai/claude-sonnet-4-6@default\n      vertex_count_tokens_location: asia-southeast1\n";

#[derive(Debug, Default, Deserialize)]
struct Raw {
    #[serde(default)]
    model_list: Vec<RawDeployment>,
}

#[derive(Debug, Default, Deserialize)]
struct RawDeployment {
    #[serde(default)]
    litellm_params: PartnerCountOptions,
}

// the loader's own view parses the key until the section registry carries an entry for it
fn load(source: &str) -> Result<PartnerCountOptions, ConfigError> {
    let (raw, _unknown) = parse_view::<Raw>(source)?;
    Ok(raw
        .model_list
        .into_iter()
        .next()
        .map(|deployment| deployment.litellm_params)
        .unwrap_or_default())
}

#[test]
fn a_configured_count_tokens_location_reaches_the_options() {
    let options = load(CONFIGURED).expect("the loader's view parses the key");

    assert_eq!(
        options.vertex_count_tokens_location.as_deref(),
        Some("asia-southeast1")
    );
}

#[test]
fn an_unset_count_tokens_location_stays_absent() {
    let options = load(
        "model_list:\n  - litellm_params:\n      model: vertex_ai/claude-sonnet-4-6@default\n",
    )
    .expect("a config without the key loads");

    assert_eq!(options, PartnerCountOptions::default());
}

#[test]
fn a_null_count_tokens_location_stays_absent() {
    let options =
        load("model_list:\n  - litellm_params:\n      vertex_count_tokens_location: null\n")
            .expect("a null key takes the default");

    assert_eq!(options.vertex_count_tokens_location, None);
}

#[test]
fn a_non_text_count_tokens_location_names_the_field() {
    let error =
        load("model_list:\n  - litellm_params:\n      vertex_count_tokens_location: [us-east5]\n")
            .expect_err("a list is not a location");

    assert!(
        error.to_string().contains("vertex_count_tokens_location"),
        "{error}"
    );
}
