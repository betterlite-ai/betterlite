#![cfg(test)]

mod body;
mod direct;
mod errors;
mod headers;
mod signing;
mod support;

use betterlite_config::sections::openai_credentials::Settings as OpenAiCredentialsSettings;
use betterlite_models::decisions::Json;
use betterlite_openai::credentials::LiteLlmGlobals;
use rstest::rstest;
use secrecy::SecretString;

use super::openai::OpenAiSearchConfig;

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the expected JSON parses")
}

fn to_json<T: serde::Serialize>(value: &T) -> Json {
    serde_json::from_slice(&serde_json::to_vec(value).expect("the value encodes"))
        .expect("the value is JSON")
}

fn openai_search_config() -> OpenAiSearchConfig {
    OpenAiSearchConfig::new(
        OpenAiCredentialsSettings::default(),
        LiteLlmGlobals {
            api_base: None,
            api_key: Some(SecretString::from("sk-global")),
            openai_key: None,
            organization: None,
        },
        Some(SecretString::from("sk-env")),
        None,
    )
}

// The five keys of a search body, in the shape the OpenAI transform builds them
const SEARCH_BODY: &str = r#"{"query":["a","b"],"filters":null,"max_num_results":2,"ranking_options":null,"rewrite_query":false}"#;
const TEXT_SEARCH_BODY: &str = r#"{"query":"a","filters":null,"max_num_results":null,"ranking_options":null,"rewrite_query":null}"#;

#[rstest]
#[case::sync_timeout_extra_body(body::BodyCase::SyncTimeoutExtraBody)]
#[case::sync_timeout_no_extra_body(body::BodyCase::SyncTimeoutNoExtraBody)]
#[case::sync_empty_extra_body(body::BodyCase::SyncEmptyExtraBody)]
#[case::sync_without_timeout(body::BodyCase::SyncWithoutTimeout)]
#[case::async_timeout_extra_body(body::BodyCase::AsyncTimeoutExtraBody)]
#[case::async_timeout_no_extra_body(body::BodyCase::AsyncTimeoutNoExtraBody)]
#[case::async_empty_extra_body(body::BodyCase::AsyncEmptyExtraBody)]
#[case::async_without_timeout(body::BodyCase::AsyncWithoutTimeout)]
#[tokio::test]
async fn search_http_body(#[case] case: body::BodyCase) {
    body::search_body_case(case).await;
}

#[rstest]
#[case::ordinary(signing::SigningCase::Ordinary)]
#[case::signed_body(signing::SigningCase::SignedBody(b"signed-body"))]
#[case::empty_signed_body(signing::SigningCase::SignedBody(b""))]
#[case::sync_query_embedding(signing::SigningCase::SyncQueryEmbedding)]
#[case::async_query_embedding(signing::SigningCase::AsyncQueryEmbedding)]
#[case::query_embedding_without_executor(signing::SigningCase::QueryEmbeddingNoExecutor)]
#[case::async_aws_signing(signing::SigningCase::AsyncAwsSigning)]
#[case::async_plain_signing(signing::SigningCase::AsyncPlainSigning)]
#[case::sync_aws_signing(signing::SigningCase::SyncAwsSigning)]
#[tokio::test]
async fn search_signing_and_embedding(#[case] case: signing::SigningCase) {
    signing::signing_case(case).await;
}

#[rstest]
#[case::sync(direct::DirectCase::SyncSearch)]
#[case::asynchronous(direct::DirectCase::AsyncSearch)]
#[case::sync_failure(direct::DirectCase::SyncFailure)]
#[case::async_failure(direct::DirectCase::AsyncFailure)]
#[tokio::test]
async fn direct_search_bypasses_http(#[case] case: direct::DirectCase) {
    direct::direct_case(case).await;
}

#[rstest]
#[case::sync_timeout(errors::ErrorCase::SyncTimeout)]
#[case::async_timeout(errors::ErrorCase::AsyncTimeout)]
#[case::status(errors::ErrorCase::Status)]
#[case::bad_request(errors::ErrorCase::BadRequest)]
#[case::wrong_type(errors::ErrorCase::WrongType)]
#[case::non_utf8(errors::ErrorCase::NonUtf8)]
#[case::transport(errors::ErrorCase::Transport)]
#[case::transform(errors::ErrorCase::Transform)]
#[tokio::test]
async fn search_timeout_and_errors(#[case] case: errors::ErrorCase) {
    errors::errors_case(case).await;
}
