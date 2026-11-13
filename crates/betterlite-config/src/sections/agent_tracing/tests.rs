#![cfg(test)]

use std::collections::BTreeMap;
use std::fs::{read, read_dir};
use std::io::Error;

use rstest::rstest;
use secrecy::ExposeSecret as Secret;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::agent_tracing::{Settings, decode_agent_tracing};

fn settings(source: &str, variables: &[(&str, &str)]) -> Settings {
    let lookup: BTreeMap<String, String> = variables
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
        .load(source)
        .unwrap()
        .sections
        .agent_tracing
}

fn load_error(source: &str) -> ConfigError {
    ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load(source)
    .err()
    .unwrap()
}

#[rstest]
#[case::absent("general_settings: {}\n", false)]
#[case::clickhouse("general_settings:\n  tracing:\n    store: clickhouse\n", true)]
#[case::another_string("general_settings:\n  tracing:\n    store: other\n", false)]
#[case::capitalized("general_settings:\n  tracing:\n    store: ClickHouse\n", false)]
#[case::empty_object("general_settings:\n  tracing: {}\n", false)]
#[case::explicit_null("general_settings:\n  tracing: null\n", false)]
#[case::non_string_store("general_settings:\n  tracing:\n    store: 5\n", false)]
fn loads_store(#[case] source: &str, #[case] enabled: bool) {
    let loaded = settings(source, &[]);
    assert_eq!(loaded.enabled, enabled);
}

#[rstest]
#[case::every_variable(
    &[
        ("CLICKHOUSE_URL", "http://clickhouse:8123"),
        ("CLICKHOUSE_READER_URL", "http://reader:8123"),
        ("CLICKHOUSE_DATABASE", "traces"),
    ],
    Some("http://clickhouse:8123"),
    Some("http://reader:8123"),
    "traces"
)]
#[case::absent_defaults(&[], None, None, "litellm")]
#[case::reader_defaults_to_url(
    &[("CLICKHOUSE_URL", "http://clickhouse:8123")],
    Some("http://clickhouse:8123"),
    Some("http://clickhouse:8123"),
    "litellm"
)]
#[case::explicit_empty_values(
    &[("CLICKHOUSE_URL", ""), ("CLICKHOUSE_READER_URL", ""), ("CLICKHOUSE_DATABASE", "")],
    Some(""),
    Some(""),
    ""
)]
fn loads_clickhouse_environment(
    #[case] variables: &[(&str, &str)],
    #[case] url: Option<&str>,
    #[case] reader_url: Option<&str>,
    #[case] database: &str,
) {
    let loaded = settings("general_settings: {}\n", variables);
    assert_eq!(
        loaded.clickhouse_url.as_ref().map(Secret::expose_secret),
        url
    );
    assert_eq!(
        loaded
            .clickhouse_reader_url
            .as_ref()
            .map(Secret::expose_secret),
        reader_url
    );
    assert_eq!(loaded.clickhouse_database, database);
}

#[test]
fn rejects_an_unknown_key_under_tracing() {
    let error = load_error("general_settings:\n  tracing:\n    store: clickhouse\n    other: 1\n");
    let ConfigError::Unsupported { paths } = error else {
        panic!("expected unsupported keys, got {error:?}");
    };
    // serde_ignored marks the Option wrapper in a path with a question mark
    assert_eq!(paths, ["general_settings.tracing.?.other"]);
}

#[test]
fn decodes_the_tracing_store() {
    let enabled =
        decode_agent_tracing(b"general_settings:\n  tracing:\n    store: clickhouse\n").unwrap();
    assert!(enabled.enabled);
    let disabled = decode_agent_tracing(b"general_settings: {}\n").unwrap();
    assert!(!disabled.enabled);
}

#[test]
fn replays_the_agent_tracing_corpus() {
    read_dir("fuzz/corpus/agent_tracing")
        .unwrap()
        .try_for_each(|entry_result| {
            let entry = entry_result?;
            let bytes = read(entry.path())?;
            let _settings = decode_agent_tracing(&bytes);
            Ok::<(), Error>(())
        })
        .unwrap();
}
