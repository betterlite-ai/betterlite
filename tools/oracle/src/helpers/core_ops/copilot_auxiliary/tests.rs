#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use serde_json::{Value, json};

use super::{Input, Op};

fn seed_source(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../fuzz/corpus/helper_case")
        .join(format!("copilot_auxiliary_{name}"));
    fs::read_to_string(&path).expect("seed should be readable")
}

fn input(name: &str) -> Input {
    let case: HelperCase =
        serde_saphyr::from_str(&seed_source(name)).expect("seed should parse as a helper case");
    serde_json::from_value(case.call.input).expect("the seed input has the typed shape")
}

fn answer(name: &str) -> Value {
    Op::run(&input(name)).expect("the helper should answer")
}

#[test]
fn replays_the_copilot_auxiliary_seeds() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../fuzz/corpus/helper_case");
    let seeds: Vec<(String, String)> = fs::read_dir(corpus)
        .expect("the helper corpus should exist")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("copilot_auxiliary_"))
        })
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("a seed has a name")
                .to_owned();
            let source = fs::read_to_string(path).expect("a seed is readable");
            (name, source)
        })
        .collect();
    assert!(!seeds.is_empty(), "no copilot_auxiliary seed exists");
    assert!(seeds.iter().all(|(name, source)| {
        let case: HelperCase =
            serde_saphyr::from_str(source).expect("a seed parses as a helper case");
        validate_helper(Path::new("<helper>"), &case).is_ok()
            && Op::check(&case.call.input)
                .unwrap_or_else(|error| panic!("{name} must call copilot_auxiliary: {error}"))
                .is_ok()
    }));
}

#[test]
fn the_scalar_input_becomes_a_list_and_the_prefix_is_stripped() {
    let answer = answer("embedding_scalar_and_prefix");

    assert_eq!(answer["body"]["model"], json!("text-embedding-3-small"));
    assert_eq!(answer["body"]["input"], json!(["hello"]));
    assert_eq!(
        answer["url"],
        json!("https://api.githubcopilot.com/embeddings")
    );
    assert_eq!(
        answer["headers"]["Authorization"],
        json!("Bearer stored-key")
    );
}

#[test]
fn optional_parameters_overlay_the_constructed_body() {
    let answer = answer("embedding_optional_override");

    assert_eq!(answer["body"]["model"], json!("override"));
    assert_eq!(answer["body"]["input"], json!(["given"]));
    assert_eq!(answer["body"]["dimensions"], json!(64_u64));
    assert_eq!(answer["body"]["user"], json!("u"));
    assert!(answer["body"].get("drop_me").is_none());
}

#[test]
fn a_falsey_key_is_an_authentication_error() {
    let error = Op::run(&input("key_missing")).expect_err("a falsey key must fail");

    assert_eq!(error.status, Some(401));
}

#[test]
fn messages_use_the_session_host_and_not_the_caller_base() {
    let answer = answer("messages_untrusted_base");

    assert_eq!(answer["api_base"], json!("https://session.test"));
    assert_eq!(answer["url"], json!("https://session.test/v1/messages"));
    assert_eq!(
        answer["headers"]["Authorization"],
        json!("Bearer session-token")
    );
}

#[test]
fn messages_keep_the_expanded_betas_verbatim() {
    let answer = answer("messages_beta_passthrough");

    assert_eq!(
        answer["headers"]["anthropic-beta"],
        json!(
            "compact-2026-01-12,context-management-2025-06-27,existing-beta,fast-mode-2026-02-01"
        )
    );
    assert_eq!(answer["headers"]["openai-intent"], json!("messages-proxy"));
    assert_eq!(
        answer["headers"]["x-github-api-version"],
        json!("2026-06-01")
    );
}

#[test]
fn a_base_that_already_names_messages_is_not_suffixed_twice() {
    let answer = answer("messages_suffix");

    assert_eq!(answer["url"], json!("https://session.test/v1/messages"));
}
