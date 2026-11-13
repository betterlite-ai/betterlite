#![cfg(test)]
use super::{GenericApiArgs, GenericApiSettings, GenericApiSettingsError, LogFormat, resolve};
use crate::generic_api_logger::tests::{ENDPOINT, args, capture_logs, environment};
use indexmap::IndexMap;
use std::collections::BTreeMap;
use std::time::Duration;
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
    // sumologic's json sets the same Content-Type, and the given value must win by exact key
    given.headers = Some(IndexMap::from([(
        "Content-Type".to_owned(),
        "text/plain".to_owned(),
    )]));
    let env = environment(&[("SUMOLOGIC_WEBHOOK_URL", "https://json.test")]);
    let settings = configured(given, &env).expect("settings");
    assert_eq!(settings.endpoint, ENDPOINT);
    assert_eq!(
        settings.event_types,
        Some(vec!["llm_api_success".to_owned()])
    );
    assert_eq!(settings.log_format, LogFormat::Single);
    assert_eq!(
        settings.headers.get("Content-Type").map(String::as_str),
        Some("text/plain")
    );
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
}

#[test]
fn an_unknown_name_warns_and_an_empty_endpoint_falls_back_to_the_environment() {
    let (logs, _guard) = capture_logs();
    let mut arguments = args();
    arguments.callback_name = Some("my_hook".to_owned());
    let settings = configured(arguments, &BTreeMap::new()).expect("settings");
    assert_eq!(settings.endpoint, ENDPOINT);
    assert!(
        logs.contents()
            .contains("callback_name 'my_hook' not found in generic_api_compatible_callbacks.json")
    );
    let mut fallback = args();
    fallback.endpoint = Some(String::new());
    let env = BTreeMap::from([(
        "GENERIC_LOGGER_ENDPOINT".to_owned(),
        "https://env.test".to_owned(),
    )]);
    assert_eq!(
        configured(fallback, &env).expect("settings").endpoint,
        "https://env.test"
    );
}
#[test]
fn a_missing_endpoint_and_a_bad_format_fail_with_the_litellm_text() {
    let mut arguments = args();
    arguments.endpoint = None;
    let Err(missing) = configured(arguments, &BTreeMap::new()) else {
        panic!("no endpoint is an error");
    };
    assert_eq!(
        missing.to_string(),
        "endpoint not set for GenericAPILogger, GENERIC_LOGGER_ENDPOINT not found in environment variables"
    );
    let mut bad = args();
    bad.log_format = Some("table".to_owned());
    let Err(invalid) = configured(bad, &BTreeMap::new()) else {
        panic!("a bad format is an error");
    };
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
    assert_eq!(settings.timeout, Duration::from_secs(9));
    let mut positive = args();
    positive.retry_delay = Some(2.5_f64);
    let delay = configured(positive, &BTreeMap::new()).expect("settings");
    assert_eq!(delay.retry_delay.to_bits(), 2.5_f64.to_bits());
    assert_eq!(delay.timeout, Duration::from_secs(600));
}
