#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_db::database_tokens::endpoint::IAMEndpoint;

use super::Settings;
use crate::loader::{ConfigError, ConfigLoader};

const WRITER: [(&str, &str); 5] = [
    ("DATABASE_HOST", "db.example"),
    ("DATABASE_USER", "svc"),
    ("DATABASE_NAME", "litellm"),
    ("DATABASE_PORT", "6543"),
    ("DATABASE_SCHEMA", "tenant"),
];

fn loader(environment: &[(&str, &str)]) -> ConfigLoader<BTreeMap<String, String>> {
    let lookup = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    ConfigLoader { lookup }
}

fn load(environment: &[(&str, &str)]) -> Settings {
    loader(environment)
        .load("model_list: []\n")
        .expect("the config loads")
        .sections
        .database_refresh_loop
}

fn load_error(environment: &[(&str, &str)]) -> ConfigError {
    loader(environment)
        .load("model_list: []\n")
        .expect_err("the config fails")
}

fn with_reader(url: &str) -> Vec<(&str, &str)> {
    let mut environment = WRITER.to_vec();
    environment.push(("DATABASE_URL_READ_REPLICA", url));
    environment
}

#[test]
fn loads_writer_endpoint() {
    let settings = load(&WRITER);
    let endpoint = settings.writer_endpoint();
    assert_eq!(endpoint.host.as_deref(), Some("db.example"));
    assert_eq!(endpoint.port, "6543");
    assert_eq!(endpoint.user.as_deref(), Some("svc"));
    assert_eq!(endpoint.name.as_deref(), Some("litellm"));
    assert_eq!(endpoint.schema.as_deref(), Some("tenant"));
}

#[test]
fn explicit_reader_endpoint_wins() {
    let settings = load(&with_reader(
        "postgresql://reader:pw@replica.example:5433/litellm?schema=reporting",
    ));
    let reader = settings
        .explicit_reader_endpoint()
        .expect("the reader URL parses")
        .expect("a reader endpoint is selected");
    assert_eq!(
        reader,
        IAMEndpoint {
            host: "replica.example".to_owned(),
            port: "5433".to_owned(),
            user: "reader".to_owned(),
            name: "litellm".to_owned(),
            schema: Some("reporting".to_owned()),
        }
    );
    // the reader URL never reaches into the writer endpoint fields
    assert_eq!(
        settings.writer_endpoint().host.as_deref(),
        Some("db.example")
    );
    assert_eq!(settings.writer_endpoint().port, "6543");
}

#[test]
fn empty_reader_url_selects_no_reader() {
    let settings = load(&with_reader(""));
    assert_eq!(
        settings
            .explicit_reader_endpoint()
            .expect("empty is not a parse error"),
        None
    );
}

#[test]
fn endpoint_missing_fields_order() {
    let host_only = load(&[("DATABASE_HOST", "db.example")]);
    assert_eq!(
        host_only
            .writer_endpoint()
            .to_iam_endpoint()
            .expect_err("user and name are missing")
            .to_string(),
        "Cannot mint a database token: DATABASE_USER, DATABASE_NAME unset. Set them so the connection URL can be reassembled around a freshly minted token."
    );
    let name_only = load(&[("DATABASE_NAME", "litellm")]);
    assert_eq!(
        name_only
            .writer_endpoint()
            .to_iam_endpoint()
            .expect_err("host and user are missing")
            .to_string(),
        "Cannot mint a database token: DATABASE_HOST, DATABASE_USER unset. Set them so the connection URL can be reassembled around a freshly minted token."
    );
    let none = load(&[]);
    assert_eq!(
        none.writer_endpoint()
            .to_iam_endpoint()
            .expect_err("every field is missing")
            .to_string(),
        "Cannot mint a database token: DATABASE_HOST, DATABASE_USER, DATABASE_NAME unset. Set them so the connection URL can be reassembled around a freshly minted token."
    );
}

#[test]
fn absent_port_5432() {
    let settings = load(&[("DATABASE_HOST", "db.example")]);
    assert_eq!(settings.writer_endpoint().port, "5432");
}

#[test]
fn empty_port_not_default() {
    let settings = load(&[("DATABASE_PORT", "")]);
    assert_eq!(settings.writer_endpoint().port, "");
}

#[test]
fn reader_url_parse_error_is_reported() {
    let settings = load(&with_reader("postgresql://reader@[::1/litellm"));
    settings
        .explicit_reader_endpoint()
        .expect_err("a malformed reader URL is a typed error");
}

#[test]
fn invalid_token_flag_stops_loading() {
    assert_eq!(
        load_error(&[("IAM_TOKEN_DB_AUTH", "maybe")]).category(),
        "environment"
    );
}
