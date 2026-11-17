use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use secrecy::{ExposeSecret as _, SecretString};

use crate::finops::pointfive::logger::client::UploadClientSource;
use crate::finops::pointfive::logger::error::PointFiveError;

use super::support::{FakeHttp, MutableEnvironment, default_params, service};

fn environment(pairs: &[(&str, &str)]) -> Arc<dyn EnvironmentLookup + Send + Sync> {
    Arc::new(MutableEnvironment::with(pairs))
}

fn resolved_key(
    params: &PointFiveInitParams,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
) -> Result<String, PointFiveError> {
    let http = FakeHttp::default();
    service(&http, environment)
        .upload_client(params)
        .map(|client| client.api_key.expose_secret().to_owned())
}

fn params_with_key(key: &str) -> PointFiveInitParams {
    let mut params = default_params();
    params.api_key = Some(SecretString::from(key));
    params
}

#[test]
fn the_configured_key_wins_over_the_environment() {
    let key = resolved_key(
        &params_with_key("configured-key"),
        environment(&[("POINTFIVE_API_KEY", "environment-key")]),
    )
    .expect("the key resolves");
    assert_eq!(key, "configured-key");
}

#[test]
fn the_environment_key_is_used_when_the_params_have_none() {
    let key = resolved_key(
        &PointFiveInitParams::default(),
        environment(&[("POINTFIVE_API_KEY", "environment-key")]),
    )
    .expect("the key resolves");
    assert_eq!(key, "environment-key");
}

#[test]
fn a_literal_key_is_used_as_is() {
    let key =
        resolved_key(&params_with_key("literal-key"), environment(&[])).expect("the key resolves");
    assert_eq!(key, "literal-key");
}

#[test]
fn an_environment_reference_is_resolved_from_the_environment() {
    let key = resolved_key(
        &params_with_key("os.environ/POINTFIVE_CONFIGURED_KEY"),
        environment(&[("POINTFIVE_CONFIGURED_KEY", "resolved-key")]),
    )
    .expect("the key resolves");
    assert_eq!(key, "resolved-key");
}

#[test]
fn an_unresolved_environment_reference_is_a_missing_key() {
    let error = resolved_key(
        &params_with_key("os.environ/POINTFIVE_CONFIGURED_KEY"),
        environment(&[]),
    )
    .err()
    .expect("the reference resolves to nothing");
    assert!(matches!(error, PointFiveError::MissingApiKey));
}

#[test]
fn an_empty_configured_key_falls_back_to_the_environment() {
    let key = resolved_key(
        &params_with_key(""),
        environment(&[("POINTFIVE_API_KEY", "environment-key")]),
    )
    .expect("the key resolves");
    assert_eq!(key, "environment-key");
}

#[test]
fn an_empty_environment_key_counts_as_unset() {
    let error = resolved_key(
        &PointFiveInitParams::default(),
        environment(&[("POINTFIVE_API_KEY", "")]),
    )
    .err()
    .expect("the key is missing");
    assert!(matches!(error, PointFiveError::MissingApiKey));
}

#[test]
fn a_missing_key_reports_the_value_error_message() {
    let error = resolved_key(&PointFiveInitParams::default(), environment(&[]))
        .err()
        .expect("the key is missing");
    assert_eq!(
        error.to_string(),
        "pointfive logging requires an api key. Set POINTFIVE_API_KEY, or litellm_settings.pointfive_params.api_key in config.yaml"
    );
}

#[test]
fn an_environment_key_that_disappears_is_missing_again() {
    let environment = MutableEnvironment::with(&[("POINTFIVE_API_KEY", "environment-key")]);
    let http = FakeHttp::default();
    let service = service(&http, Arc::new(environment.clone()));
    assert!(
        service
            .upload_client(&PointFiveInitParams::default())
            .is_ok()
    );
    environment.unset("POINTFIVE_API_KEY");
    assert!(matches!(
        service.upload_client(&PointFiveInitParams::default()),
        Err(PointFiveError::MissingApiKey)
    ));
}

#[test]
fn the_upload_client_takes_its_retries_from_the_params() {
    let mut params = params_with_key("configured-key");
    params.max_upload_retries = 5;
    let http = FakeHttp::default();
    let client = service(&http, environment(&[]))
        .upload_client(&params)
        .expect("the client builds");
    assert_eq!(client.max_retries, 5);
}
