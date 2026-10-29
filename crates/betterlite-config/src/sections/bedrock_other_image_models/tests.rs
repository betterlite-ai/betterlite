#![cfg(test)]

use std::collections::BTreeMap;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

type Read = fn(&Settings) -> Option<String>;

fn load(
    params: &str,
    settings: &str,
    environment: &[(&str, &str)],
) -> Result<Settings, ConfigError> {
    let source = format!(
        "model_list:\n- model_name: a\n  litellm_params: {{model: bedrock/a{params}}}\nlitellm_settings: {{{settings}}}\n"
    );
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    let config = ConfigLoader { lookup }.load(&source)?;
    Ok(config.sections.bedrock_other_image_models)
}

#[rstest::rstest]
#[case::loads_cfg_scale("cfg_scale", "CFG_SCALE", |settings: &Settings| {
    settings.cfg_scale.map(|value| value.to_string())
})]
#[case::loads_seed("seed", "SEED", |settings: &Settings| {
    settings.seed.map(|value| value.to_string())
})]
#[case::loads_steps("steps", "STEPS", |settings: &Settings| {
    settings.steps.map(|value| value.to_string())
})]
#[case::loads_width("width", "WIDTH", |settings: &Settings| {
    settings.width.map(|value| value.to_string())
})]
#[case::loads_height("height", "HEIGHT", |settings: &Settings| {
    settings.height.map(|value| value.to_string())
})]
fn bedrock_image_models_config_cases(
    #[case] key: &str,
    #[case] variable: &str,
    #[case] read: Read,
) {
    assert_defaults_to_none(key, read);
    assert_reads_the_global_value(key, read);
    assert_reads_the_deployment_value(key, read);
    assert_deployment_overrides_global(key, read);
    assert_resolves_an_environment_reference(key, variable, read);
    assert_rejects_an_unrelated_key(key);
}

fn assert_defaults_to_none(key: &str, read: Read) {
    let defaults = load("", "", &[]).unwrap();
    assert_eq!(read(&defaults), None, "{key} default");
}

fn assert_reads_the_global_value(key: &str, read: Read) {
    let global_only = load("", &format!("{key}: 3"), &[]).unwrap();
    assert_eq!(read(&global_only), Some("3".to_owned()), "{key} global");
}

fn assert_reads_the_deployment_value(key: &str, read: Read) {
    let deployment_only = load(&format!(", {key}: 5"), "", &[]).unwrap();
    assert_eq!(
        read(&deployment_only),
        Some("5".to_owned()),
        "{key} deployment"
    );
}

fn assert_deployment_overrides_global(key: &str, read: Read) {
    let both = load(&format!(", {key}: 5"), &format!("{key}: 3"), &[]).unwrap();
    assert_eq!(read(&both), Some("5".to_owned()), "{key} precedence");
}

fn assert_resolves_an_environment_reference(key: &str, variable: &str, read: Read) {
    let environment = [(variable, "9")];
    let referenced = load("", &format!("{key}: os.environ/{variable}"), &environment).unwrap();
    assert_eq!(read(&referenced), Some("9".to_owned()), "{key} environment");
}

fn assert_rejects_an_unrelated_key(key: &str) {
    let error = load(&format!(", {key}_unrelated: 1"), "", &[]).unwrap_err();
    assert!(
        matches!(&error, ConfigError::Unsupported { paths } if *paths == [format!("model_list.0.litellm_params.{key}_unrelated")]),
        "{error}"
    );
}

#[test]
fn second_deployment_wins_over_the_first() {
    let source = "model_list:\n- model_name: a\n  litellm_params: {model: bedrock/a, cfg_scale: 5}\n- model_name: b\n  litellm_params: {model: bedrock/b, cfg_scale: 7}\n";
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap();
    assert_eq!(
        config.sections.bedrock_other_image_models.cfg_scale,
        Some(7_i64)
    );
}
