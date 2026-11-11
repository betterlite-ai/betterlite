#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use secrecy::ExposeSecret as Secret;

use crate::loader::{ConfigError, ConfigLoader};
use crate::sections::agent_tracing::{
    DATABASE_VARIABLE, DEFAULT_DATABASE, READER_URL_VARIABLE, Settings, URL_VARIABLE,
    decode_agent_tracing,
};

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
        (URL_VARIABLE, "http://clickhouse:8123"),
        (READER_URL_VARIABLE, "http://reader:8123"),
        (DATABASE_VARIABLE, "traces"),
    ],
    Some("http://clickhouse:8123"),
    Some("http://reader:8123"),
    "traces"
)]
#[case::absent_defaults(&[], None, None, DEFAULT_DATABASE)]
#[case::reader_defaults_to_url(
    &[(URL_VARIABLE, "http://clickhouse:8123")],
    Some("http://clickhouse:8123"),
    Some("http://clickhouse:8123"),
    DEFAULT_DATABASE
)]
#[case::explicit_empty_values(
    &[(URL_VARIABLE, ""), (READER_URL_VARIABLE, ""), (DATABASE_VARIABLE, "")],
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
    assert_eq!(paths, ["general_settings.tracing.other"]);
}

#[rstest]
fn replays_the_agent_tracing_corpus(#[files("fuzz/corpus/agent_tracing/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let _settings = decode_agent_tracing(&bytes);
}
