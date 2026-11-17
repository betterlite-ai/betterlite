use std::sync::Arc;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::pointfive_params::PointFiveInitParams;
use secrecy::SecretString;

use crate::finops::pointfive::logger::client::UploadClientSource as _;
use crate::finops::pointfive::types::DEFAULT_API_URL;

use super::support::{FakeHttp, MutableEnvironment, default_params, service};

fn environment(pairs: &[(&str, &str)]) -> Arc<dyn EnvironmentLookup + Send + Sync> {
    Arc::new(MutableEnvironment::with(pairs))
}

fn resolved_url(
    params: &PointFiveInitParams,
    environment: Arc<dyn EnvironmentLookup + Send + Sync>,
) -> String {
    let http = FakeHttp::default();
    service(&http, environment)
        .upload_client(params)
        .expect("the client builds")
        .api_url
}

fn params_with_url(url: &str) -> PointFiveInitParams {
    let mut params = default_params();
    params.api_key = Some(SecretString::from("configured-key"));
    params.api_url = Some(url.to_owned());
    params
}

fn params_with_key() -> PointFiveInitParams {
    let mut params = default_params();
    params.api_key = Some(SecretString::from("configured-key"));
    params
}

#[test]
fn the_configured_url_wins_over_the_environment() {
    let url = resolved_url(
        &params_with_url("https://configured.example/ingest"),
        environment(&[("POINTFIVE_API_URL", "https://environment.example/ingest")]),
    );
    assert_eq!(url, "https://configured.example/ingest");
}

#[test]
fn the_environment_url_is_used_when_the_params_have_none() {
    let url = resolved_url(
        &params_with_key(),
        environment(&[("POINTFIVE_API_URL", "https://environment.example/ingest")]),
    );
    assert_eq!(url, "https://environment.example/ingest");
}

#[test]
fn the_default_url_is_used_when_nothing_is_configured() {
    let url = resolved_url(&params_with_key(), environment(&[]));
    assert_eq!(url, DEFAULT_API_URL);
}

#[test]
fn an_unresolved_environment_reference_falls_through_to_the_environment() {
    let url = resolved_url(
        &params_with_url("os.environ/POINTFIVE_CONFIGURED_URL"),
        environment(&[("POINTFIVE_API_URL", "https://environment.example/ingest")]),
    );
    assert_eq!(url, "https://environment.example/ingest");
}

#[test]
fn an_environment_reference_resolves_to_its_environment_value() {
    let url = resolved_url(
        &params_with_url("os.environ/POINTFIVE_CONFIGURED_URL"),
        environment(&[(
            "POINTFIVE_CONFIGURED_URL",
            "https://reference.example/ingest",
        )]),
    );
    assert_eq!(url, "https://reference.example/ingest");
}

#[test]
fn an_empty_environment_url_falls_through_to_the_default() {
    let url = resolved_url(
        &params_with_key(),
        environment(&[("POINTFIVE_API_URL", "")]),
    );
    assert_eq!(url, DEFAULT_API_URL);
}

#[test]
fn an_empty_configured_url_falls_back_to_the_environment() {
    let url = resolved_url(
        &params_with_url(""),
        environment(&[("POINTFIVE_API_URL", "https://environment.example/ingest")]),
    );
    assert_eq!(url, "https://environment.example/ingest");
}
