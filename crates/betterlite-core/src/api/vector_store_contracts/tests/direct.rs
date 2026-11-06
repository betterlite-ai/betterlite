use std::time::Duration;

use betterlite_http::error::HttpError;
use betterlite_openai::chat::deployment::OpenAiDefaults;
use betterlite_openai::embedding::{EmbeddingError, EmbeddingResponse};
use betterlite_provider::api::vector_store_contracts::config::{
    EmbeddingEntry, Params, VectorStoreEmbeddingExecutor as _,
};
use betterlite_provider::api::vector_store_contracts::error::VectorStoreEmbeddingError;
use rstest::rstest;
use secrecy::SecretString;
use tokio::io::AsyncReadExt as _;
use tokio::time::timeout;

use super::{
    DirectEmbeddingExecutor, direct_executor, executor_over, listen, params, provider,
    request_body, text,
};

#[tokio::test]
async fn direct_executor_posts_query_embedding_and_returns_the_mock_vector() {
    let (listener, base) = listen().await;
    let executor = direct_executor(&base);
    let configuration = params(r#"{"dimensions": 2}"#);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &configuration
        )
    );
    assert!(sent.starts_with("POST /v1/embeddings HTTP/1.1\r\n"));
    assert!(sent.contains("authorization: Bearer sk-test"));
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"], "dimensions": 2}"#)
    );
    let response: EmbeddingResponse = answered.expect("embedding");
    assert_eq!(response.object, "list");
    assert_eq!(response.model, "text-embedding-3-small");
    assert_eq!(response.data.first().map(|record| record.index), Some(0));
    assert_eq!(response.usage.prompt_tokens, 1);
    assert_eq!(response.usage.total_tokens, 1);
}

#[tokio::test]
async fn direct_executor_aembed_posts_the_same_request() {
    let (listener, base) = listen().await;
    let executor = direct_executor(&base);
    let configuration = params(r#"{"dimensions": 2}"#);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(
            EmbeddingEntry::Aembedding,
            "openai/text-embedding-3-small",
            "hello",
            &configuration
        )
    );
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"], "dimensions": 2}"#)
    );
    assert_eq!(answered.expect("embedding").object, "list");
}

#[rstest]
#[case::model("model")]
#[case::input("input")]
#[tokio::test]
async fn direct_executor_rejects_explicit_arguments_in_the_configuration(#[case] name: &str) {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = Params::from([(name.to_owned(), text("given"))]);
    let error = executor
        .embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &configuration,
        )
        .await
        .expect_err("duplicate argument");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::DuplicateArgument { name: field } if field.as_str() == name
    ));
}

#[tokio::test]
async fn direct_executor_keeps_metadata_out_of_the_provider_body() {
    let (listener, base) = listen().await;
    let executor = direct_executor(&base);
    let configuration = params(r#"{"dimensions": 2, "metadata": {"keep": "yes"}}"#);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &configuration
        )
    );
    let _answered = answered.expect("embedding");
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"], "dimensions": 2}"#)
    );
}

async fn embed_through(executor: &DirectEmbeddingExecutor, given: &str) -> (String, Params) {
    let (listener, base) = listen().await;
    let mut configuration = params(given);
    configuration
        .entry("api_base".to_owned())
        .or_insert_with(|| text(&base));
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &configuration
        )
    );
    let _answered = answered.expect("embedding");
    let body = request_body(&sent);
    (sent, body)
}

#[tokio::test]
async fn direct_executor_sends_unknown_parameters_in_the_provider_body() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let (_sent, body) = embed_through(&executor, r#"{"input_type": "search"}"#).await;
    assert_eq!(
        body,
        params(
            r#"{"model": "text-embedding-3-small", "input": ["hello"], "input_type": "search"}"#
        )
    );
}

#[tokio::test]
async fn direct_executor_merges_extra_body_into_the_provider_body() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let (_sent, body) =
        embed_through(&executor, r#"{"extra_body": {"foo": 1}, "user": "u1"}"#).await;
    assert_eq!(
        body,
        params(
            r#"{"model": "text-embedding-3-small", "input": ["hello"], "user": "u1", "foo": 1}"#
        )
    );
}

#[tokio::test]
async fn direct_executor_accepts_litellm_call_parameters_without_sending_them() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = r#"{"max_retries": 2, "num_retries": 3, "caching": true, "drop_params": true, "api_version": "v1", "api_type": "openai", "custom_llm_provider": "openai", "litellm_call_id": "c", "logger_fn": "f", "extra_headers": {"x-extra": "1"}, "headers": {"x-header": "2"}}"#;
    let (_sent, body) = embed_through(&executor, configuration).await;
    assert_eq!(
        body,
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"]}"#)
    );
}

#[tokio::test]
async fn direct_executor_keeps_litellm_owned_parameters_out_of_the_provider_body() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = r#"{"organization": "org", "base_url": "http://other", "deployment_id": "d", "default_headers": {"x": "1"}, "request_timeout": 5, "litellm_metadata": {"team": "a"}, "tags": ["t"], "model_info": {"id": "m"}, "ssl_verify": false, "input_cost_per_token": 0.1, "allowed_openai_params": ["dimensions"], "client_secret": "s", "azure_password": "p", "mock_response": [0.5], "_litellm_internal": 1}"#;
    let (_sent, body) = embed_through(&executor, configuration).await;
    assert_eq!(
        body,
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"]}"#)
    );
}

#[tokio::test]
async fn direct_executor_leaves_a_null_provider_parameter_out_of_the_body() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let (_sent, body) = embed_through(&executor, r#"{"input_type": null}"#).await;
    assert_eq!(
        body,
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"]}"#)
    );
}

#[tokio::test]
async fn direct_executor_keeps_other_providers_credentials_out_of_the_body() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = r#"{"aws_secret_access_key": "k", "aws_region_name": "r", "vertex_project": "p", "vertex_credentials": "c", "aws_foo": 1, "vertex_x": 2, "hf_model_name": "h"}"#;
    let (_sent, body) = embed_through(&executor, configuration).await;
    assert_eq!(
        body,
        params(r#"{"model": "text-embedding-3-small", "input": ["hello"]}"#)
    );
}

#[rstest]
#[case::dimensions(r#"{"additional_drop_params": ["dimensions"], "dimensions": 2}"#, "{}")]
#[case::user_and_encoding_format(
    r#"{"additional_drop_params": ["user"], "user": "u", "encoding_format": "float"}"#,
    r#"{"encoding_format": "float"}"#
)]
#[case::encoding_format_stays(
    r#"{"additional_drop_params": ["encoding_format"], "encoding_format": "float"}"#,
    r#"{"encoding_format": "float"}"#
)]
#[case::encoding_format_none_is_no_format(
    r#"{"additional_drop_params": ["encoding_format"], "encoding_format": "none"}"#,
    "{}"
)]
#[case::encoding_format_none_unlisted(r#"{"encoding_format": " None "}"#, "{}")]
#[case::unknown_parameter_stays(
    r#"{"additional_drop_params": ["input_type"], "input_type": "x"}"#,
    r#"{"input_type": "x"}"#
)]
#[tokio::test]
async fn direct_executor_drops_only_the_optional_parameters_additional_drop_params_names(
    #[case] given: &str,
    #[case] kept: &str,
) {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let (_sent, body) = embed_through(&executor, given).await;
    let mut expected = params(r#"{"model": "text-embedding-3-small", "input": ["hello"]}"#);
    expected.extend(params(kept));
    assert_eq!(body, expected);
}

#[tokio::test]
async fn direct_executor_names_additional_drop_params_of_the_wrong_type() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = params(r#"{"additional_drop_params": "dimensions"}"#);
    let error = executor
        .embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &configuration,
        )
        .await
        .expect_err("wrong type");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::FieldType {
            field: "additional_drop_params",
            ..
        }
    ));
}

#[rstest]
#[case::known_openai_embedding_model("text-embedding-3-small", "{}", "text-embedding-3-small")]
#[case::named_provider("some-model", r#"{"custom_llm_provider": "openai"}"#, "some-model")]
#[case::provider_already_in_the_model(
    "openai/some-model",
    r#"{"custom_llm_provider": "openai"}"#,
    "some-model"
)]
#[tokio::test]
async fn direct_executor_resolves_the_openai_provider_like_get_llm_provider(
    #[case] model: &str,
    #[case] given: &str,
    #[case] sent_model: &str,
) {
    let (listener, base) = listen().await;
    let executor = executor_over(Vec::new(), OpenAiDefaults::default());
    let mut configuration = params(given);
    configuration.insert("api_base".to_owned(), text(&base));
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(EmbeddingEntry::Embedding, model, "hello", &configuration)
    );
    let _answered = answered.expect("embedding");
    assert_eq!(
        request_body(&sent),
        params(&format!(
            r#"{{"model": "{sent_model}", "input": ["hello"]}}"#
        ))
    );
}

#[tokio::test]
async fn direct_executor_refuses_a_model_no_provider_serves_before_any_request() {
    let executor = executor_over(Vec::new(), OpenAiDefaults::default());
    let error = executor
        .embed(
            EmbeddingEntry::Embedding,
            "some-model",
            "hello",
            &Params::new(),
        )
        .await
        .expect_err("provider");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::Embedding {
            source: EmbeddingError::UnsupportedProvider { .. }
        }
    ));
}

#[tokio::test]
async fn direct_executor_waits_600_seconds_for_a_model_no_deployment_serves() {
    let executor = executor_over(
        Vec::new(),
        OpenAiDefaults {
            api_bases: Vec::new(),
            api_keys: Vec::new(),
            request_timeout: Some(7.0_f64),
            organization: None,
        },
    );
    let unmatched = executor
        .deployment("openai/some-new-model", &Params::new())
        .expect("deployment");
    assert_eq!(unmatched.timeout, Duration::from_secs(600));
    assert!((unmatched.timeout_seconds - 600.0).abs() < f64::EPSILON);
}

#[tokio::test]
async fn direct_executor_forwards_encoding_format_and_user() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let (_sent, body) =
        embed_through(&executor, r#"{"encoding_format": "float", "user": "u1"}"#).await;
    assert_eq!(
        body,
        params(
            r#"{"model": "text-embedding-3-small", "input": ["hello"], "encoding_format": "float", "user": "u1"}"#
        )
    );
}

#[tokio::test]
async fn direct_executor_calls_a_model_no_deployment_serves() {
    let executor = executor_over(Vec::new(), OpenAiDefaults::default());
    let (listener, base) = listen().await;
    let configuration = Params::from([
        ("api_base".to_owned(), text(&base)),
        ("api_key".to_owned(), text("sk-config")),
    ]);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(
            EmbeddingEntry::Embedding,
            "openai/some-new-model",
            "hello",
            &configuration
        )
    );
    let _answered = answered.expect("embedding");
    assert!(sent.contains("authorization: Bearer sk-config"));
    assert_eq!(
        request_body(&sent),
        params(r#"{"model": "some-new-model", "input": ["hello"]}"#)
    );
}

#[tokio::test]
async fn direct_executor_falls_back_to_the_environment_defaults_without_a_deployment() {
    let (listener, base) = listen().await;
    let executor = executor_over(
        Vec::new(),
        OpenAiDefaults {
            api_bases: vec![base],
            api_keys: vec![SecretString::from("sk-default")],
            request_timeout: None,
            organization: None,
        },
    );
    let configuration = Params::new();
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(
            EmbeddingEntry::Embedding,
            "openai/some-new-model",
            "hello",
            &configuration
        )
    );
    let _answered = answered.expect("embedding");
    assert!(sent.contains("authorization: Bearer sk-default"));
}

#[tokio::test]
async fn direct_executor_keeps_the_deployment_when_options_are_null() {
    let (listener, base) = listen().await;
    let executor = direct_executor(&base);
    let configuration = params(r#"{"api_base": null, "api_key": null, "timeout": null}"#);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &configuration
        )
    );
    let _answered = answered.expect("embedding");
    assert!(sent.contains("authorization: Bearer sk-test"));
}

#[tokio::test]
async fn direct_executor_overrides_the_deployment_base_and_key() {
    let (listener, base) = listen().await;
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let configuration = Params::from([
        ("api_base".to_owned(), text(&base)),
        ("api_key".to_owned(), text("sk-override")),
    ]);
    let (sent, answered) = tokio::join!(
        provider(listener),
        executor.embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &configuration
        )
    );
    let _answered = answered.expect("embedding");
    assert!(sent.starts_with("POST /v1/embeddings HTTP/1.1\r\n"));
    assert!(sent.contains("authorization: Bearer sk-override"));
}

#[tokio::test]
async fn direct_executor_gives_up_after_the_configured_timeout() {
    let (listener, base) = listen().await;
    let silent = async {
        let (mut stream, _) = listener.accept().await.expect("connection");
        let mut received = Vec::new();
        drop(stream.read_to_end(&mut received).await);
    };
    let executor = direct_executor(&base);
    let configuration = params(r#"{"timeout": 0.2}"#);
    let waiting = executor.embed(
        EmbeddingEntry::Embedding,
        "openai/text-embedding-3-small",
        "hello",
        &configuration,
    );
    // a lost override falls back to the deployment's 6000 second wait, so the bound fails the test instead
    let ((), outcome) = tokio::join!(silent, timeout(Duration::from_secs(5), waiting));
    assert!(matches!(
        outcome.expect("the configured timeout ends the call within 5 seconds"),
        Err(VectorStoreEmbeddingError::Embedding {
            source: EmbeddingError::Http {
                source: HttpError::Timeout
            }
        })
    ));
}

#[rstest]
#[case::base("api_base", "1", "api_base", "string")]
#[case::key("api_key", "[]", "api_key", "string")]
#[case::timeout("timeout", r#""soon""#, "timeout", "number")]
#[case::extra_body("extra_body", r#""raw""#, "extra_body", "object")]
#[tokio::test]
async fn direct_executor_names_the_field_of_the_wrong_type(
    #[case] name: &str,
    #[case] value: &str,
    #[case] field: &str,
    #[case] expected: &str,
) {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let error = executor
        .embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &params(&format!(r#"{{"{name}": {value}}}"#)),
        )
        .await
        .expect_err("wrong type");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::FieldType { field: found, expected: wanted }
            if found == field && wanted == expected
    ));
}

#[tokio::test]
async fn direct_executor_rejects_a_timeout_no_wait_can_last() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let error = executor
        .embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &params(r#"{"timeout": -1}"#),
        )
        .await
        .expect_err("timeout");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::Timeout { seconds, .. } if seconds.is_sign_negative()
    ));
}

#[tokio::test]
async fn direct_executor_names_a_typed_parameter_of_the_wrong_type() {
    let executor = direct_executor("http://127.0.0.1:9/v1");
    let error = executor
        .embed(
            EmbeddingEntry::Embedding,
            "openai/text-embedding-3-small",
            "hello",
            &params(r#"{"dimensions": "two"}"#),
        )
        .await
        .expect_err("request");
    assert!(matches!(error, VectorStoreEmbeddingError::Request { .. }));
}

#[tokio::test]
async fn direct_executor_reports_defaults_that_make_no_deployment() {
    let executor = executor_over(
        Vec::new(),
        OpenAiDefaults {
            api_bases: vec![String::from("not a url")],
            api_keys: Vec::new(),
            request_timeout: None,
            organization: None,
        },
    );
    let error = executor
        .embed(
            EmbeddingEntry::Embedding,
            "openai/some-new-model",
            "hello",
            &Params::new(),
        )
        .await
        .expect_err("deployment");
    assert!(matches!(
        error,
        VectorStoreEmbeddingError::Deployment { .. }
    ));
}
