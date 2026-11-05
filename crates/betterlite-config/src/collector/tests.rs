#![cfg(test)]
use super::{
    AddressError, CollectorAddress, CollectorConfigInput, CollectorSettings, UnavailablePolicy,
    decode_collector_settings, load_collector_settings, load_spend_traceback_policy,
    parse_collector_address,
};
use rust_decimal::Decimal;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::time::Duration;

#[test]
fn loads_collector_delivery_settings() {
    let defaults = load_collector_settings(&BTreeMap::new()).unwrap();
    assert!(!defaults.enabled);
    assert_eq!(defaults.address, "unix:///var/run/litellm/collector.sock");
    assert_eq!(defaults.buffer_size, 1000);
    assert_eq!(defaults.on_unavailable, UnavailablePolicy::Fallback);
    assert_eq!(defaults.job_role, None);
    assert_eq!(
        defaults.validate().unwrap(),
        (Duration::from_secs(10), Duration::from_secs(1))
    );
}

#[test]
fn loads_case_insensitive_collector_fields() {
    let environment = BTreeMap::from([
        ("LiTeLlM_CoLlEcToR_EnAbLeD".to_owned(), "TRUE".to_owned()),
        (
            "litellm_collector_address".to_owned(),
            "tcp://localhost:1".to_owned(),
        ),
        ("litellm_collector_buffer_size".to_owned(), "2".to_owned()),
        (
            "litellm_collector_on_unavailable".to_owned(),
            "drop".to_owned(),
        ),
        (
            "litellm_collector_drain_timeout_seconds".to_owned(),
            "0.5".to_owned(),
        ),
        (
            "litellm_collector_connect_timeout_seconds".to_owned(),
            "0.25".to_owned(),
        ),
        ("litellm_job_role".to_owned(), "collector".to_owned()),
    ]);
    let settings = load_collector_settings(&environment).unwrap();
    assert!(settings.enabled);
    assert_eq!(settings.address, "tcp://localhost:1");
    assert_eq!(settings.buffer_size, 2);
    assert_eq!(settings.on_unavailable, UnavailablePolicy::Drop);
    assert_eq!(settings.job_role.as_deref(), Some("collector"));
    assert_eq!(
        settings.validate().unwrap(),
        (Duration::from_millis(500), Duration::from_millis(250))
    );
    assert_eq!(settings.drain_timeout_seconds, Decimal::new(5, 1));
    assert_eq!(settings.connect_timeout_seconds, Decimal::new(25, 2));
}

#[rstest::rstest]
#[case::zero_buffer("BUFFER_SIZE", "0")]
#[case::invalid_buffer("BUFFER_SIZE", "bad")]
#[case::zero_drain("DRAIN_TIMEOUT_SECONDS", "0")]
#[case::negative_connect("CONNECT_TIMEOUT_SECONDS", "-1")]
#[case::invalid_decimal("DRAIN_TIMEOUT_SECONDS", "bad")]
#[case::invalid_flag("ENABLED", "bad")]
#[case::invalid_policy("ON_UNAVAILABLE", "DROP")]
#[case::unknown_field("OTHER", "1")]
#[case::wrong_role_alias("JOB_ROLE", "collector")]
#[case::capacity_overflow("BUFFER_SIZE", "18446744073709551615")]
#[case::timeout_overflow("CONNECT_TIMEOUT_SECONDS", "1000000000000000")]
#[case::sub_nanosecond("DRAIN_TIMEOUT_SECONDS", "0.0000000001")]
fn rejects_invalid_settings(#[case] field: &str, #[case] value: &str) {
    let environment = BTreeMap::from([(format!("LITELLM_COLLECTOR_{field}"), value.to_owned())]);
    load_collector_settings(&environment).unwrap_err();
}

#[rstest::rstest]
#[case::yes(Some("yes"), true)]
#[case::blank_spaces(Some(" TRUE "), true)]
#[case::false_value(Some("false"), false)]
#[case::absent(None, false)]
fn loads_spend_traceback_policy_from_the_env_value(
    #[case] value: Option<&str>,
    #[case] expected: bool,
) {
    assert_eq!(
        load_spend_traceback_policy(value).unwrap().suppress,
        expected
    );
}

#[test]
fn rejects_an_invalid_spend_traceback_value() {
    load_spend_traceback_policy(Some("invalid")).unwrap_err();
}

#[rstest::rstest]
#[case::ipv6("tcp://[::1]:123", CollectorAddress::Tcp {host: "::1".to_owned(), port: 123})]
#[case::local_case("tcp://LOCALHOST:0", CollectorAddress::Tcp {host: "localhost".to_owned(), port: 0})]
#[case::ipv4("tcp://127.5.4.3:65535", CollectorAddress::Tcp {host: "127.5.4.3".to_owned(), port: u16::MAX})]
#[case::unix_path("unix://ignored/a/../b%20c", CollectorAddress::Unix {path: "/a/../b%20c".to_owned()})]
#[case::relative_unix("unix:relative?query", CollectorAddress::Unix {path: "relative".to_owned()})]
fn preserves_ipv6_and_unix_path_spelling(
    #[case] address: &str,
    #[case] expected: CollectorAddress,
) {
    assert_eq!(parse_collector_address(address).unwrap(), expected);
}

#[rstest::rstest]
#[case::port("tcp://localhost:bad")]
#[case::range("tcp://localhost:65536")]
#[case::ipv6("tcp://[bad]:10")]
fn propagates_url_syntax_errors(#[case] address: &str) {
    assert!(matches!(
        parse_collector_address(address),
        Err(AddressError::Syntax(_))
    ));
}

#[rstest::rstest]
#[case::remote("tcp://example.com:1")]
#[case::remote_ip("tcp://192.168.0.1:1")]
#[case::no_port("tcp://localhost")]
#[case::no_path("unix://host")]
#[case::no_scheme("invalid")]
#[case::scheme("https://localhost:1")]
fn rejects_semantic_address_errors(#[case] address: &str) {
    assert!(matches!(
        parse_collector_address(address),
        Err(AddressError::RemoteHost | AddressError::Invalid)
    ));
}

#[rstest::rstest]
fn replays_the_collector_settings_corpus(
    #[files("fuzz/corpus/collector_settings/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    if let Ok(input) = decode_collector_settings(&bytes) {
        match input {
            CollectorConfigInput::Settings { config } => {
                let _address = parse_collector_address(&config.address);
            }
            CollectorConfigInput::Address { value } => {
                let _address = parse_collector_address(&value);
            }
            CollectorConfigInput::Suppression { value } => {
                let _policy = load_spend_traceback_policy(value.as_deref());
            }
        }
    }
}

#[test]
fn empty_environment_values_take_defaults() {
    let environment = BTreeMap::from([
        ("LITELLM_COLLECTOR_ENABLED".to_owned(), String::new()),
        ("LITELLM_COLLECTOR_BUFFER_SIZE".to_owned(), String::new()),
    ]);
    assert_eq!(
        load_collector_settings(&environment).unwrap(),
        CollectorSettings::default()
    );
    decode_collector_settings(br#"{"kind":"settings","config":{"buffer_size":0}}"#).unwrap_err();
    decode_collector_settings(b"invalid").unwrap_err();
}

#[rstest::rstest]
#[case::absent(br#"{"kind":"settings","config":{}}"#)]
#[case::null(br#"{"kind":"settings","config":{"enabled":null,"address":null,"buffer_size":null,"on_unavailable":null,"drain_timeout_seconds":null,"connect_timeout_seconds":null,"job_role":null}}"#)]
fn null_and_absent_settings_take_defaults(#[case] bytes: &[u8]) {
    let CollectorConfigInput::Settings { config } = decode_collector_settings(bytes).unwrap()
    else {
        panic!("expected settings");
    };
    assert_eq!(config, CollectorSettings::default());
}

#[test]
fn rejects_duplicate_environment_aliases() {
    let environment = BTreeMap::from([
        ("LITELLM_COLLECTOR_ENABLED".to_owned(), "true".to_owned()),
        ("litellm_collector_enabled".to_owned(), "false".to_owned()),
    ]);
    load_collector_settings(&environment).unwrap_err();
}
