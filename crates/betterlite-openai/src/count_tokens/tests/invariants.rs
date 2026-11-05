#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use rstest::rstest;
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

#[rstest]
fn replays_the_openai_count_tokens_corpus(
    #[files("fuzz/corpus/openai_count_tokens/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let decoded = decode_input_token_count(&bytes);
    match name.split('-').next().unwrap() {
        "valid" => assert!(matches!(decoded, Ok(Some(_))), "{name}"),
        "null" => assert!(matches!(decoded, Ok(None)), "{name}"),
        "scalar" => assert!(
            matches!(decoded, Err(CountTokensError::NotAnObject)),
            "{name}"
        ),
        "malformed" | "truncated" => {
            assert!(
                matches!(decoded, Err(CountTokensError::Decode { .. })),
                "{name}"
            );
        }
        other => panic!("unknown corpus category {other}"),
    }
}
