#![cfg(test)]

use std::collections::BTreeMap;

use rstest::rstest;

use crate::loader::{ConfigError, ConfigLoader};

#[rstest]
#[case::loads_default_max_tokens("", &[], Ok((false, 4096)))]
#[case::loads_default_max_tokens("", &[("DEFAULT_MAX_TOKENS", " +100 ")], Ok((false, 100)))]
#[case::loads_default_max_tokens("", &[("DEFAULT_MAX_TOKENS", "4k")], Err("DEFAULT_MAX_TOKENS"))]
#[case::loads_default_max_tokens(
    "litellm_settings:\n  DEFAULT_MAX_TOKENS: 5\n",
    &[],
    Err("litellm_settings.DEFAULT_MAX_TOKENS")
)]
#[case::loads_modify_params("litellm_settings:\n  modify_params: true\n", &[], Ok((true, 4096)))]
#[case::loads_modify_params(
    "litellm_settings:\n  modify_params: false\n",
    &[("LITELLM_MODIFY_PARAMS", "1")],
    Ok((false, 4096))
)]
#[case::loads_modify_params("litellm_settings:\n  modify_params: 'false'\n", &[], Ok((false, 4096)))]
#[case::loads_modify_params("litellm_settings:\n  modify_params: [true]\n", &[], Err("yaml"))]
#[case::loads_litellm_modify_params("", &[("LITELLM_MODIFY_PARAMS", "false")], Ok((false, 4096)))]
#[case::loads_litellm_modify_params("", &[("LITELLM_MODIFY_PARAMS", "")], Ok((false, 4096)))]
#[case::loads_litellm_modify_params("", &[("LITELLM_MODIFY_PARAMS", " ")], Ok((false, 4096)))]
fn provider_base_config_cases(
    #[case] source: &str,
    #[case] environment: &[(&str, &str)],
    #[case] expected: Result<(bool, i64), &str>,
) {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let loaded = ConfigLoader { lookup }.load(source);
    match (loaded, expected) {
        (Ok(config), Ok(settings)) => {
            let litellm = config.litellm_settings;
            assert_eq!(
                (litellm.modify_params, litellm.default_max_tokens),
                settings
            );
        }
        (Err(ConfigError::Environment { source: env_error }), Err(expected_path)) => {
            assert!(env_error.to_string().contains(expected_path));
        }
        (Err(ConfigError::Unsupported { paths }), Err(expected_path)) => {
            assert_eq!(paths, [expected_path]);
        }
        (Err(ConfigError::Yaml { .. }), Err("yaml")) => {}
        (outcome, expectation) => panic!("{outcome:?} does not match {expectation:?}"),
    }
}

#[test]
fn shared_env_modify_params() {
    let configured = ConfigLoader {
        lookup: BTreeMap::from([("LITELLM_MODIFY_PARAMS".to_owned(), "maybe".to_owned())]),
    }
    .load("litellm_settings: {modify_params: true}")
    .unwrap();
    assert!(configured.litellm_settings.modify_params);
    let yes = ConfigLoader {
        lookup: BTreeMap::from([("LITELLM_MODIFY_PARAMS".to_owned(), " Yes ".to_owned())]),
    }
    .load("")
    .unwrap();
    assert!(yes.litellm_settings.modify_params);
    let empty = ConfigLoader {
        lookup: BTreeMap::from([("LITELLM_MODIFY_PARAMS".to_owned(), String::new())]),
    }
    .load("")
    .unwrap();
    assert!(!empty.litellm_settings.modify_params);
    let error = ConfigLoader {
        lookup: BTreeMap::from([(
            "LITELLM_MODIFY_PARAMS".to_owned(),
            "maybe-secret".to_owned(),
        )]),
    }
    .load("")
    .unwrap_err()
    .to_string();
    assert!(error.contains("LITELLM_MODIFY_PARAMS"));
    assert!(!error.contains("maybe-secret"));
}

#[test]
fn shared_env_default_max_tokens() {
    let parsed = ConfigLoader {
        lookup: BTreeMap::from([("DEFAULT_MAX_TOKENS".to_owned(), " 123 ".to_owned())]),
    }
    .load("")
    .unwrap();
    assert_eq!(parsed.litellm_settings.default_max_tokens, 123);
    let empty = ConfigLoader {
        lookup: BTreeMap::from([("DEFAULT_MAX_TOKENS".to_owned(), String::new())]),
    }
    .load("")
    .unwrap();
    assert_eq!(empty.litellm_settings.default_max_tokens, 4096);
    let error = ConfigLoader {
        lookup: BTreeMap::from([("DEFAULT_MAX_TOKENS".to_owned(), "1_000".to_owned())]),
    }
    .load("")
    .unwrap_err()
    .to_string();
    assert!(error.contains("DEFAULT_MAX_TOKENS"));
    assert!(!error.contains("1_000"));
}

#[test]
fn shared_env_os_environ_number() {
    let source = "model_list:\n  - model_name: x\n    litellm_params:\n      timeout: os.environ/REQUEST_TIMEOUT\n";
    let parsed = super::map_loader(BTreeMap::from([(
        "REQUEST_TIMEOUT".to_owned(),
        " 1e3 ".to_owned(),
    )]))
    .load(source)
    .unwrap();
    assert_eq!(
        parsed.model_list.first().unwrap().litellm_params.timeout,
        Some(1_000.0_f64)
    );
    let empty = super::map_loader(BTreeMap::from([(
        "REQUEST_TIMEOUT".to_owned(),
        String::new(),
    )]))
    .load(source)
    .unwrap();
    assert_eq!(empty.model_list[0].litellm_params.timeout, None);
    let error = super::map_loader(BTreeMap::from([(
        "REQUEST_TIMEOUT".to_owned(),
        "1_000".to_owned(),
    )]))
    .load(source)
    .unwrap_err()
    .to_string();
    assert!(error.contains("REQUEST_TIMEOUT"));
    assert!(!error.contains("1_000"));
}
