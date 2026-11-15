#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_secrets::gateway_key::get_litellm_gateway_api_key;
use pyo3::prelude::*;
use tempfile::TempDir;

use super::token::TokenFileStore;
use super::{ProxyClientNative, register};

fn store_in(home: &Path) -> TokenFileStore {
    TokenFileStore {
        token_file: home.join(".litellm").join("token.json"),
    }
}

fn stored_home(contents: &str) -> TempDir {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".litellm");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("token.json"), contents).unwrap();
    home
}

#[test]
fn redacts_a_bearer_token_with_the_shared_patterns() {
    let native = ProxyClientNative::new().unwrap();
    assert_eq!(
        native.redact_text("header Authorization: Bearer abcdefghijklmnop"),
        "header Authorization: REDACTED"
    );
}

#[test]
fn leaves_a_plain_sentence_alone() {
    let native = ProxyClientNative::new().unwrap();
    assert_eq!(
        native.redact_text("the server is ready"),
        "the server is ready"
    );
}

#[test]
fn register_adds_the_proxy_client_class() {
    Python::initialize();
    Python::attach(|py| {
        let module = PyModule::new(py, "probe").unwrap();
        register(&module).unwrap();
        assert!(module.hasattr("ProxyClientNative").unwrap());
    });
}

#[test]
fn native_gateway_key_reads_the_token_file_under_home() {
    let home = stored_home(r#"{"base_url": "http://proxy:4000", "key": "sk-stored"}"#);
    let key = temp_env::with_var("HOME", Some(home.path()), || {
        ProxyClientNative::gateway_api_key(Some("http://proxy:4000"))
    });
    assert_eq!(key.as_deref(), Some("sk-stored"));
}

#[test]
fn native_gateway_key_is_none_when_home_has_no_token_file() {
    let home = TempDir::new().unwrap();
    let key = temp_env::with_var("HOME", Some(home.path()), || {
        ProxyClientNative::gateway_api_key(None)
    });
    assert_eq!(key, None);
}

#[test]
fn gateway_key_comes_from_the_token_file() {
    let home = stored_home(r#"{"base_url": "http://proxy:4000", "key": "sk-stored"}"#);
    let key = get_litellm_gateway_api_key(Some("http://proxy:4000"), &store_in(home.path()));
    assert_eq!(key.as_deref(), Some("sk-stored"));
}

#[test]
fn gateway_key_ignores_a_trailing_slash_on_the_expected_url() {
    let home = stored_home(r#"{"base_url": "http://proxy:4000", "key": "sk-stored"}"#);
    let key = get_litellm_gateway_api_key(Some("http://proxy:4000/"), &store_in(home.path()));
    assert_eq!(key.as_deref(), Some("sk-stored"));
}

#[test]
fn gateway_key_is_none_for_another_server() {
    let home = stored_home(r#"{"base_url": "http://proxy:4000", "key": "sk-stored"}"#);
    let key = get_litellm_gateway_api_key(Some("http://elsewhere:4000"), &store_in(home.path()));
    assert_eq!(key, None);
}

#[test]
fn gateway_key_is_none_without_a_token_file() {
    let home = TempDir::new().unwrap();
    let key = get_litellm_gateway_api_key(Some("http://proxy:4000"), &store_in(home.path()));
    assert_eq!(key, None);
}

#[test]
fn gateway_key_is_none_for_an_invalid_token_file() {
    let home = stored_home("not json");
    let key = get_litellm_gateway_api_key(Some("http://proxy:4000"), &store_in(home.path()));
    assert_eq!(key, None);
}

#[test]
fn gateway_key_is_none_when_the_file_holds_no_key() {
    let home = stored_home(r#"{"base_url": "http://proxy:4000"}"#);
    let key = get_litellm_gateway_api_key(Some("http://proxy:4000"), &store_in(home.path()));
    assert_eq!(key, None);
}

#[test]
fn gateway_key_is_none_when_the_token_path_is_a_directory() {
    let home = TempDir::new().unwrap();
    let directory = home.path().join(".litellm");
    fs::create_dir_all(directory.join("token.json")).unwrap();
    let key = get_litellm_gateway_api_key(Some("http://proxy:4000"), &store_in(home.path()));
    assert_eq!(key, None);
}

#[test]
fn gateway_key_is_none_when_the_base_url_is_not_a_string() {
    let home = stored_home(r#"{"base_url": 7, "key": "sk-stored"}"#);
    let key = get_litellm_gateway_api_key(Some("http://proxy:4000"), &store_in(home.path()));
    assert_eq!(key, None);
}

#[test]
fn gateway_key_without_an_expected_url_returns_the_stored_key() {
    let home = stored_home(r#"{"base_url": "http://proxy:4000", "key": "sk-stored"}"#);
    let key = get_litellm_gateway_api_key(None, &store_in(home.path()));
    assert_eq!(key.as_deref(), Some("sk-stored"));
}
