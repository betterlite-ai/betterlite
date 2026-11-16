use betterlite_guardrails::repello_lifecycle::config::{
    RepelloAIParams, RepelloAISettings, RepelloLifecycleError,
};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use crate::support::{ASSET_ID, empty_env, lookup, params};

fn params_with_key(api_key: Option<&str>) -> RepelloAIParams {
    let mut configured = params();
    configured.api_key = api_key.map(SecretString::from);
    configured
}

#[test]
fn an_explicit_api_key_wins_over_both_environment_names() {
    let env = lookup(&[
        ("ARGUS_API_KEY", "argus-env"),
        ("REPELLOAI_API_KEY", "repello-env"),
    ]);
    let settings = RepelloAISettings::resolve(&params_with_key(Some("explicit")), &env).unwrap();
    assert_eq!(settings.api_key.expose_secret(), "explicit");
}

#[test]
fn the_argus_environment_name_wins_over_repelloai() {
    let env = lookup(&[
        ("ARGUS_API_KEY", "argus-env"),
        ("REPELLOAI_API_KEY", "repello-env"),
    ]);
    let settings = RepelloAISettings::resolve(&params_with_key(None), &env).unwrap();
    assert_eq!(settings.api_key.expose_secret(), "argus-env");
}

#[test]
fn the_repelloai_environment_name_is_the_last_key_source() {
    let env = lookup(&[("REPELLOAI_API_KEY", "repello-env")]);
    let settings = RepelloAISettings::resolve(&params_with_key(None), &env).unwrap();
    assert_eq!(settings.api_key.expose_secret(), "repello-env");
}

#[rstest]
#[case::no_key(None, &[])]
#[case::empty_parameter(Some(""), &[("ARGUS_API_KEY", "")])]
#[case::empty_argus_falls_to_repello(None, &[("ARGUS_API_KEY", "")])]
fn a_missing_key_raises_litellm_s_missing_secrets_error(
    #[case] api_key: Option<&str>,
    #[case] entries: &[(&str, &str)],
) {
    let env = lookup(entries);
    let error = RepelloAISettings::resolve(&params_with_key(api_key), &env).unwrap_err();
    assert!(matches!(&error, RepelloLifecycleError::MissingApiKey));
    assert_eq!(
        error.to_string(),
        "Couldn't get Repello API key. Set `ARGUS_API_KEY` in the environment or pass `api_key` to the guardrail in the config file."
    );
}

#[test]
fn an_empty_repelloai_key_is_still_no_key() {
    let env = lookup(&[("REPELLOAI_API_KEY", "")]);
    let error = RepelloAISettings::resolve(&params_with_key(None), &env).unwrap_err();
    assert!(matches!(error, RepelloLifecycleError::MissingApiKey));
}

#[test]
fn a_missing_key_is_reported_before_a_missing_asset_id() {
    let mut configured = params_with_key(None);
    configured.asset_id = None;
    let error = RepelloAISettings::resolve(&configured, &empty_env()).unwrap_err();
    assert!(matches!(error, RepelloLifecycleError::MissingApiKey));
}

#[test]
fn an_empty_asset_id_raises_litellm_s_value_error() {
    let mut configured = params_with_key(Some("explicit"));
    configured.asset_id = Some(String::new());
    let error = RepelloAISettings::resolve(&configured, &empty_env()).unwrap_err();
    assert!(matches!(&error, RepelloLifecycleError::MissingAssetId));
    assert_eq!(
        error.to_string(),
        "Repello guardrail requires an `asset_id`. Create an asset in the Repello dashboard and set `asset_id` on the guardrail in the config file."
    );
}

#[test]
fn a_resolved_asset_id_is_kept_as_given() {
    let mut configured = params_with_key(Some("explicit"));
    configured.asset_id = Some(ASSET_ID.to_owned());
    let settings = RepelloAISettings::resolve(&configured, &empty_env()).unwrap();
    assert_eq!(settings.asset_id, ASSET_ID);
}
