#![cfg(test)]
#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "each case helper states one LiteLLM behavior as a single scenario"
    )
)]

use std::collections::BTreeMap;

use rstest::rstest;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

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
    Ok(config.sections.bedrock_other_titan_embeddings)
}

fn settings_of(result: Result<Settings, ConfigError>) -> Settings {
    result.expect("the config loads")
}

#[rstest]
#[case::loads_normalize(loads_normalize)]
#[case::loads_dimensions(loads_dimensions)]
fn titan_embeddings_config_cases(#[case] check: fn()) {
    check();
}

fn loads_normalize() {
    // both class settings default to None, not the AWS documentation defaults of true and 1024
    assert_eq!(
        settings_of(load("", "", &[])),
        Settings {
            normalize: None,
            dimensions: None
        }
    );

    let global = settings_of(load("", "normalize: true", &[]));
    assert_eq!(global.normalize, Some(true));

    // deployment litellm_params win over global litellm_settings, and an explicit false counts
    let deployment = settings_of(load(", normalize: false", "", &[]));
    assert_eq!(deployment.normalize, Some(false));
    let precedence = settings_of(load(", normalize: true", "normalize: false", &[]));
    assert_eq!(precedence.normalize, Some(true));

    let referenced = settings_of(load(
        "",
        "normalize: os.environ/TITAN_NORMALIZE",
        &[("TITAN_NORMALIZE", "false")],
    ));
    assert_eq!(referenced.normalize, Some(false));

    // an empty variable counts as unset
    let empty = settings_of(load(
        "",
        "normalize: os.environ/TITAN_NORMALIZE",
        &[("TITAN_NORMALIZE", "")],
    ));
    assert_eq!(empty.normalize, None);

    let missing = load("", "normalize: os.environ/TITAN_NORMALIZE", &[]).unwrap_err();
    assert!(matches!(
        &missing,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "TITAN_NORMALIZE" && path == "litellm_settings.normalize"
    ));

    // normalize is a bool, so a number or a bare string is a wrong type rather than a flag
    for literal in ["1", "\"yes\""] {
        let wrong = load("", &format!("normalize: {literal}"), &[]).unwrap_err();
        assert!(
            matches!(
                &wrong,
                ConfigError::InvalidScalar { path, .. } if path == "litellm_settings.normalize"
            ),
            "{literal}: {wrong}"
        );
    }

    assert_rejects_unrelated_keys("normalize");
}

fn loads_dimensions() {
    assert_eq!(settings_of(load("", "", &[])).dimensions, None);

    let global = settings_of(load("", "dimensions: 256", &[]));
    assert_eq!(global.dimensions, Some(256));

    let deployment = settings_of(load(", dimensions: 512", "", &[]));
    assert_eq!(deployment.dimensions, Some(512));
    let precedence = settings_of(load(", dimensions: 512", "dimensions: 256", &[]));
    assert_eq!(precedence.dimensions, Some(512));

    let referenced = settings_of(load(
        "",
        "dimensions: os.environ/TITAN_DIMENSIONS",
        &[("TITAN_DIMENSIONS", "1024")],
    ));
    assert_eq!(referenced.dimensions, Some(1024));

    let empty = settings_of(load(
        "",
        "dimensions: os.environ/TITAN_DIMENSIONS",
        &[("TITAN_DIMENSIONS", "")],
    ));
    assert_eq!(empty.dimensions, None);

    let missing = load("", "dimensions: os.environ/TITAN_DIMENSIONS", &[]).unwrap_err();
    assert!(matches!(
        &missing,
        ConfigError::MissingEnvironment { variable, path }
            if variable == "TITAN_DIMENSIONS" && path == "litellm_settings.dimensions"
    ));

    // the declared type is int, so a negative value parses where an unsigned one would not
    assert_eq!(
        settings_of(load("", "dimensions: -1", &[])).dimensions,
        Some(-1)
    );

    let not_a_number = load("", "dimensions: wide", &[]).unwrap_err();
    assert!(
        matches!(
            &not_a_number,
            ConfigError::InvalidInteger { path, .. } if path == "litellm_settings.dimensions"
        ),
        "{not_a_number}"
    );

    assert_rejects_unrelated_keys("dimensions");
}

fn assert_rejects_unrelated_keys(key: &str) {
    let deployment = load(&format!(", {key}_unrelated: 1"), "", &[]).unwrap_err();
    assert!(
        matches!(
            &deployment,
            ConfigError::Unsupported { paths }
                if *paths == [format!("model_list.0.litellm_params.{key}_unrelated")]
        ),
        "{deployment}"
    );

    let global = load("", &format!("{key}_unrelated: 1"), &[]).unwrap_err();
    assert!(
        matches!(
            &global,
            ConfigError::Unsupported { paths }
                if *paths == [format!("litellm_settings.{key}_unrelated")]
        ),
        "{global}"
    );
}

#[test]
fn second_deployment_wins_over_the_first() {
    let source = "model_list:\n- model_name: a\n  litellm_params: {model: bedrock/a, normalize: true, dimensions: 256}\n- model_name: b\n  litellm_params: {model: bedrock/b, normalize: false, dimensions: 512}\n";
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .unwrap();
    assert_eq!(
        config.sections.bedrock_other_titan_embeddings,
        Settings {
            normalize: Some(false),
            dimensions: Some(512)
        }
    );
}

#[test]
fn deployment_and_global_settings_stay_separate() {
    let settings = settings_of(load(", dimensions: 512", "normalize: true", &[]));
    assert_eq!(
        settings,
        Settings {
            normalize: Some(true),
            dimensions: Some(512)
        }
    );
}
