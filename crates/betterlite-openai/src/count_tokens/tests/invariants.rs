#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

use secrecy::SecretString;

use super::super::error::CountTokensError;
use super::super::handler::{HandlerRequest, decode_input_token_count};
use super::super::request::ResponsesInput;
use super::{OK, counter, run};

#[tokio::test]
async fn openai_count_tokens_credential_identity_isolation() {
    let scenario = |key: &str, env: &str| {
        format!(
            r#"{{"model":"m","key":"{key}","env_key":"{env}","messages":[{{"role":"user","content":"hi"}}]}}"#
        )
    };
    let (_, first) = run(&scenario("sk-one", "sk-env-one"), 200, OK).await;
    let (_, second) = run(&scenario("sk-two", "sk-env-two"), 200, OK).await;
    let (_, env_only) = run(&scenario("", "sk-env-two"), 200, OK).await;
    assert_eq!(first.unwrap().0, "Bearer sk-one");
    assert_eq!(second.unwrap().0, "Bearer sk-two");
    assert_eq!(env_only.unwrap().0, "Bearer sk-env-two");
}

#[tokio::test]
async fn openai_count_tokens_secret_safe_failure() {
    let secret = "sk-very-secret-fixture";
    let key = SecretString::from(secret.to_owned());
    let counter = counter(Some(secret.to_owned()));
    let input = ResponsesInput::Text(String::from("hi"));
    let request = HandlerRequest {
        model: "m",
        input: &input,
        api_key: &key,
        api_base: Some("http://127.0.0.1:1/v1"),
        timeout: None,
        tools: None,
        instructions: None,
    };
    let outbound = counter.handler.prepare(&request).unwrap();
    let error = counter
        .handler
        .handle_count_tokens_request(&request)
        .await
        .unwrap_err();
    let scenario = format!(
        r#"{{"model":"m","key":"{secret}","messages":[{{"role":"user","content":"hi"}}]}}"#
    );
    let (rendered, _) = run(&scenario, 500, "boom").await;
    let everything = [
        format!("{counter:?}"),
        format!("{request:?}"),
        format!("{outbound:?}"),
        format!("{error:?}"),
        error.to_string(),
        rendered,
    ]
    .join("\n");
    assert!(!everything.contains(secret), "{everything}");
}

fn replays(path: &Path) -> bool {
    let bytes = fs::read(path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let decoded = decode_input_token_count(&bytes);
    match name.split('-').next().unwrap() {
        "valid" => matches!(decoded, Ok(Some(_))),
        "null" => matches!(decoded, Ok(None)),
        "scalar" => matches!(decoded, Err(CountTokensError::NotAnObject)),
        "malformed" | "truncated" => matches!(decoded, Err(CountTokensError::Decode { .. })),
        other => panic!("unknown corpus category {other}"),
    }
}

#[test]
fn replays_the_openai_count_tokens_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_count_tokens");
    let seeds: Vec<PathBuf> = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert!(!seeds.is_empty());
    assert_eq!(seeds.iter().find(|path| !replays(path)), None);
}
