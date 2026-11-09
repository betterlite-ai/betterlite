#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use indexmap::IndexMap;

use super::{GenericApiArgs, GenericApiSettings, GenericApiSettingsError, LogFormat, resolve};
use crate::generic_api_logger::tests::{ENDPOINT, args, capture_logs, environment};

type Resolved = Result<GenericApiSettings, GenericApiSettingsError>;

fn configured(arguments: GenericApiArgs, env: &BTreeMap<String, String>) -> Resolved {
    resolve(arguments, env, &IndexMap::new())
}

#[test]
fn given_values_beat_the_named_config_and_the_environment() {
    let mut given = args();
    given.callback_name = Some("sumologic".to_owned());
    given.event_types = Some(vec!["llm_api_success".to_owned()]);
    given.log_format = Some("single".to_owned());
    let env = environment(&[("SUMOLOGIC_WEBHOOK_URL", "https://json.test")]);
    let settings = configured(given, &env).expect("settings");
    assert_eq!(settings.endpoint, ENDPOINT);
    assert_eq!(
        settings.event_types,
        Some(vec!["llm_api_success".to_owned()])
    );
    assert_eq!(settings.log_format, LogFormat::Single);

    let mut named = args();
    named.callback_name = Some("sample_callback".to_owned());
    named.endpoint = None;
    let sample_env = environment(&[
        ("SAMPLE_CALLBACK_URL", "https://sample.test"),
        ("SAMPLE_CALLBACK_API_KEY", "sk-key"),
    ]);
    let sample = configured(named, &sample_env).expect("settings");
    assert_eq!(sample.endpoint, "https://sample.test");
    assert_eq!(
        sample.event_types,
        Some(vec![
            "llm_api_success".to_owned(),
            "llm_api_failure".to_owned()
        ])
    );
    assert_eq!(
        sample.headers.get("Authorization").map(String::as_str),
        Some("Bearer sk-key")
    );
    assert_eq!(sample.log_format, LogFormat::JsonArray);

    let mut fallback = args();
    fallback.endpoint = Some(String::new());
    let fallback_env = environment(&[("GENERIC_LOGGER_ENDPOINT", "https://env.test")]);
    let resolved = configured(fallback, &fallback_env).expect("settings");
    assert_eq!(resolved.endpoint, "https://env.test");
}

#[test]
fn unknown_callback_name_warns_and_keeps_the_given_values() {
    let (logs, _guard) = capture_logs();
    let mut arguments = args();
    arguments.callback_name = Some("my_hook".to_owned());
    let settings = configured(arguments, &BTreeMap::new()).expect("settings");
    assert_eq!(settings.endpoint, ENDPOINT);
    let warning = "callback_name 'my_hook' not found in generic_api_compatible_callbacks.json";
    assert!(logs.contents().contains(warning));
}

#[test]
fn a_missing_endpoint_and_a_bad_format_fail_with_the_litellm_text() {
    let mut arguments = args();
    arguments.endpoint = None;
    let missing = configured(arguments, &BTreeMap::new()).expect_err("no endpoint");
    assert_eq!(
        missing.to_string(),
        "endpoint not set for GenericAPILogger, GENERIC_LOGGER_ENDPOINT not found in environment variables"
    );
    let mut bad = args();
    bad.log_format = Some("table".to_owned());
    let invalid = configured(bad, &BTreeMap::new()).expect_err("a bad format");
    assert_eq!(
        invalid.to_string(),
        "Invalid log_format: table. Must be one of: 'json_array', 'ndjson', 'single'"
    );
}

#[test]
fn headers_merge_in_litellm_order_by_exact_key() {
    let env = environment(&[(
        "GENERIC_LOGGER_HEADERS",
        "X-Env=env, Content-Type=text/plain, broken",
    )]);
    let global = IndexMap::from([("X-Global".to_owned(), "global".to_owned())]);
    let mut arguments = args();
    arguments.callback_name = Some("sumologic".to_owned());
    arguments.headers = Some(IndexMap::from([
        ("X-Given".to_owned(), "given".to_owned()),
        ("content-type".to_owned(), "text/csv".to_owned()),
    ]));
    let settings = resolve(arguments, &env, &global).expect("settings");
    assert_eq!(settings.log_format, LogFormat::Ndjson);
    let keys = settings
        .headers
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    assert_eq!(
        keys,
        [
            "Content-Type",
            "X-Env",
            "X-Global",
            "X-Given",
            "content-type"
        ]
    );
    assert_eq!(
        settings.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        settings.headers.get("content-type").map(String::as_str),
        Some("text/csv")
    );
}

#[test]
fn retries_and_the_delay_are_clamped_and_the_timeout_is_kept() {
    let mut arguments = args();
    arguments.max_retries = Some(-4);
    arguments.retry_delay = Some(-1.5_f64);
    arguments.timeout = Some(Duration::from_secs(9));
    let settings = configured(arguments, &BTreeMap::new()).expect("settings");
    assert_eq!(settings.max_retries, 0);
    assert_eq!(settings.retry_delay.to_bits(), 0.0_f64.to_bits());
    assert_eq!(settings.timeout, Some(Duration::from_secs(9)));
    let mut positive = args();
    positive.retry_delay = Some(2.5_f64);
    let delay = configured(positive, &BTreeMap::new()).expect("settings");
    assert_eq!(delay.retry_delay.to_bits(), 2.5_f64.to_bits());
}
