use std::collections::BTreeMap;
use std::time::Duration;

use http::header::{AUTHORIZATION, CONTENT_TYPE};

use super::support::{Fixture, SPAN_URL, TRACE_URL};
use crate::opik::delivery::settings::{
    DEFAULT_BASE_URL, DEFAULT_PROJECT_NAME, LoggerOptions, OpikSettingsError, resolve_settings,
};

fn environment(entries: &[(&str, &str)]) -> BTreeMap<String, String> {
    entries
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn settings_default_the_project_name_and_base_url() {
    let settings = resolve_settings(&LoggerOptions::default(), &BTreeMap::new(), None).unwrap();
    assert_eq!(settings.project_name, DEFAULT_PROJECT_NAME);
    assert_eq!(settings.trace_url.as_str(), TRACE_URL);
    assert_eq!(settings.span_url.as_str(), SPAN_URL);
    assert_eq!(
        format!("{DEFAULT_BASE_URL}/v1/private/traces/batch"),
        TRACE_URL
    );
}

fn options() -> LoggerOptions<'static> {
    LoggerOptions {
        project_name: None,
        url: None,
        api_key: None,
        workspace: None,
        batch_size: None,
        flush_interval: None,
        max_queue_size: None,
    }
}

#[test]
fn settings_take_the_user_value_before_the_environment_and_file() {
    let mut options = options();
    options.project_name = Some("user-project");
    let environment = environment(&[("OPIK_PROJECT_NAME", "env-project")]);
    let file = "[opik]\nproject_name = file-project\n";
    let settings = resolve_settings(&options, &environment, Some(file)).unwrap();
    assert_eq!(settings.project_name, "user-project");
}

#[test]
fn settings_take_the_environment_value_before_the_file() {
    let environment = environment(&[("OPIK_PROJECT_NAME", "env-project")]);
    let file = "[opik]\nproject_name = file-project\n";
    let settings = resolve_settings(&LoggerOptions::default(), &environment, Some(file)).unwrap();
    assert_eq!(settings.project_name, "env-project");
}

#[test]
fn settings_take_the_file_value_when_the_environment_is_silent() {
    let file = "[opik]\nproject_name = file-project\nurl_override = https://opik.example/api\n";
    let settings =
        resolve_settings(&LoggerOptions::default(), &BTreeMap::new(), Some(file)).unwrap();
    assert_eq!(settings.project_name, "file-project");
    assert_eq!(
        settings.trace_url.as_str(),
        "https://opik.example/api/v1/private/traces/batch"
    );
    assert_eq!(
        settings.span_url.as_str(),
        "https://opik.example/api/v1/private/spans/batch"
    );
}

#[test]
fn settings_keep_an_explicit_empty_project_name() {
    let mut options = options();
    options.project_name = Some("");
    let settings = resolve_settings(&options, &BTreeMap::new(), None).unwrap();
    assert_eq!(settings.project_name, "");
}

#[test]
fn settings_read_the_batch_options_from_the_environment_and_the_user() {
    let environment = environment(&[
        ("DEFAULT_BATCH_SIZE", "9"),
        ("DEFAULT_FLUSH_INTERVAL_SECONDS", "11"),
    ]);
    let defaults = resolve_settings(&LoggerOptions::default(), &environment, None).unwrap();
    assert_eq!(defaults.batch.batch_size, 9);
    assert_eq!(defaults.batch.flush_interval, Duration::from_secs(11));
    assert_eq!(defaults.batch.max_queue_size, 50_000);

    let mut options = options();
    options.batch_size = Some(3);
    options.flush_interval = Some(2);
    options.max_queue_size = Some(7);
    let overridden = resolve_settings(&options, &environment, None).unwrap();
    assert_eq!(overridden.batch.batch_size, 3);
    assert_eq!(overridden.batch.flush_interval, Duration::from_secs(2));
    assert_eq!(overridden.batch.max_queue_size, 7);
}

#[test]
fn settings_build_the_workspace_and_authorization_headers() {
    let mut fixture = Fixture::new();
    fixture.options.workspace = Some("team");
    fixture.options.api_key = Some("secret-key");
    let logger = fixture.build(None);
    let headers = &logger.settings.headers;
    assert_eq!(headers.get("comet-workspace").unwrap(), "team");
    let authorization = headers.get(AUTHORIZATION).unwrap();
    assert_eq!(authorization, "secret-key");
    assert!(authorization.is_sensitive());
    assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
}

#[test]
fn settings_omit_the_optional_headers_when_unset() {
    let logger = Fixture::new().build(None);
    let headers = &logger.settings.headers;
    assert!(headers.get("comet-workspace").is_none());
    assert!(headers.get(AUTHORIZATION).is_none());
    assert_eq!(headers.get(CONTENT_TYPE).unwrap(), "application/json");
}

#[test]
fn settings_reject_a_base_url_that_cannot_be_parsed() {
    let mut fixture = Fixture::new();
    fixture.options.url = Some("not a url");
    let outcome = fixture.try_build(None);
    let Err(error) = outcome else {
        panic!("an unparsable base URL must fail")
    };
    assert!(matches!(
        error,
        OpikSettingsError::Url {
            name: "url_override",
            ..
        }
    ));
}

#[test]
fn settings_read_the_api_key_and_workspace_from_the_environment() {
    let mut fixture = Fixture::new();
    fixture.environment =
        environment(&[("OPIK_API_KEY", "env-key"), ("OPIK_WORKSPACE", "env-team")]);
    let logger = fixture.build(None);
    assert_eq!(
        logger.settings.headers.get(AUTHORIZATION).unwrap(),
        "env-key"
    );
    assert_eq!(
        logger.settings.headers.get("comet-workspace").unwrap(),
        "env-team"
    );
}
