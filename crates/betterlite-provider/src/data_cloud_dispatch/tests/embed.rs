use std::collections::BTreeMap;

use betterlite_http::proxy::UpstreamResponse;
use betterlite_prompts::arguments::json::Json as PromptJson;
use http::{HeaderMap, HeaderName, HeaderValue};

use crate::data_cloud_dispatch::call::EmbeddingReply;
use crate::data_cloud_dispatch::context::DispatchGlobals;
use crate::data_cloud_dispatch::embed::dispatch as embedding_dispatch;
use crate::data_cloud_dispatch::error::DispatchError;
use crate::data_cloud_dispatch::select::EmbeddingConfig;
use crate::snowflake::embedding::transform_embedding_response;

use super::support::{
    FixedClock, Fixture, NoTokens, OwnedGlobals, Scripted, TEST_KEY, databricks_embedding_reply,
    embedding_context, environment, ok, status,
};

#[tokio::test]
async fn databricks_embedding_sends_the_prepared_request() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("databricks-bge-large-en");
    fixture.environment = environment(&[
        (
            "DATABRICKS_API_BASE",
            "https://dbc.example.com/serving-endpoints",
        ),
        ("DATABRICKS_API_KEY", "db-key"),
    ]);
    let transport = Scripted::with_replies(vec![ok(databricks_embedding_reply())]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    let reply = embedding_dispatch(&call, EmbeddingConfig::Databricks, &context)
        .await
        .expect("the databricks embedding answers");
    let EmbeddingReply::Databricks(embedding) = reply else {
        panic!("expected a databricks embedding reply");
    };
    assert_eq!(embedding.model, "databricks-bge-large-en");
    let sent = transport.sent();
    assert_eq!(
        sent[0].url,
        "https://dbc.example.com/serving-endpoints/embeddings"
    );
    assert_eq!(
        sent[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer db-key")
    );
    let body: PromptJson = serde_json::from_slice(&sent[0].body).expect("the body is JSON");
    let PromptJson::Object(fields) = body else {
        panic!("expected a JSON object body");
    };
    assert_eq!(
        fields.get("model"),
        Some(&PromptJson::Text(String::from("databricks-bge-large-en")))
    );
    assert_eq!(
        fields.get("input"),
        Some(&PromptJson::Text(String::from("hello")))
    );
}

#[tokio::test]
async fn databricks_embedding_reports_a_status_failure() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        (
            "DATABRICKS_API_BASE",
            "https://dbc.example.com/serving-endpoints",
        ),
        ("DATABRICKS_API_KEY", "db-key"),
    ]);
    let transport = Scripted::with_replies(vec![status(500, "boom")]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    match embedding_dispatch(&call, EmbeddingConfig::Databricks, &context).await {
        Err(DispatchError::Upstream { status: 500, body }) => assert_eq!(body, "boom"),
        other => panic!("expected the upstream status, got {other:?}"),
    }
}

#[tokio::test]
async fn snowflake_embedding_uses_the_cortex_url_and_snowflake_jwt() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("openai-gpt-4.1");
    fixture.environment = environment(&[
        ("SNOWFLAKE_JWT", "pat/abc"),
        ("SNOWFLAKE_ACCOUNT_ID", "acct1"),
    ]);
    let transport = Scripted::with_replies(vec![ok(
        r#"{"object":"list","data":[{"object":"embedding","embedding":[[0.1,0.2]],"index":0}],"model":"openai-gpt-4.1","usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    let reply = embedding_dispatch(&call, EmbeddingConfig::Snowflake, &context)
        .await
        .expect("the snowflake embedding answers");
    let EmbeddingReply::Snowflake(embedding) = reply else {
        panic!("expected a snowflake embedding reply");
    };
    assert_eq!(embedding.hidden_model, "openai-gpt-4.1");
    assert_eq!(embedding.response.model, "snowflake/openai-gpt-4.1");
    let sent = transport.sent();
    assert_eq!(
        sent[0].url,
        "https://acct1.snowflakecomputing.com/api/v2/cortex/inference:embed"
    );
    assert_eq!(
        sent[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer abc")
    );
    assert_eq!(
        sent[0]
            .headers
            .get("x-snowflake-authorization-token-type")
            .and_then(|value| value.to_str().ok()),
        Some("PROGRAMMATIC_ACCESS_TOKEN")
    );
    let body: PromptJson = serde_json::from_slice(&sent[0].body).expect("the body is JSON");
    let PromptJson::Object(fields) = body else {
        panic!("expected a JSON object body");
    };
    assert_eq!(
        fields.get("model"),
        Some(&PromptJson::Text(String::from("openai-gpt-4.1")))
    );
    assert_eq!(
        fields.get("text"),
        Some(&PromptJson::Text(String::from("hello")))
    );
}

#[tokio::test]
async fn snowflake_embedding_reports_the_upstream_status() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        ("SNOWFLAKE_JWT", "pat/abc"),
        ("SNOWFLAKE_ACCOUNT_ID", "acct1"),
    ]);
    let transport = Scripted::with_replies(vec![status(400, "bad input")]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    match embedding_dispatch(&call, EmbeddingConfig::Snowflake, &context).await {
        Err(DispatchError::SnowflakeException(exception)) => {
            assert_eq!(exception.status_code, 400);
            assert_eq!(exception.message, "bad input");
        }
        other => panic!("expected the snowflake exception, got {other:?}"),
    }
}

#[tokio::test]
async fn oci_embedding_signs_the_request_and_decodes_the_reply() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("cohere.embed-english-v3.0");
    fixture.environment = environment(&[
        ("OCI_USER", "ocid1.user.oc1..bbbb"),
        ("OCI_FINGERPRINT", "aa:bb:cc"),
        ("OCI_TENANCY", "ocid1.tenancy.oc1..cccc"),
        ("OCI_KEY", TEST_KEY),
        ("OCI_COMPARTMENT_ID", "ocid1.compartment.oc1..aaaa"),
    ]);
    let transport = Scripted::with_replies(vec![ok(
        r#"{"embeddings":[[0.1,0.2]],"model_id":"cohere.embed-english-v3.0","model_version":"1.0"}"#,
    )]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    let reply = embedding_dispatch(&call, EmbeddingConfig::Oci, &context)
        .await
        .expect("the oci embedding answers");
    let EmbeddingReply::Oci(embedding) = reply else {
        panic!("expected an oci embedding reply");
    };
    assert_eq!(embedding.model, "cohere.embed-english-v3.0");
    let sent = transport.sent();
    assert_eq!(
        sent[0].url,
        "https://inference.generativeai.us-ashburn-1.oci.oraclecloud.com/20231130/actions/embedText"
    );
    assert!(sent[0].headers.contains_key("authorization"));
    assert!(sent[0].headers.contains_key("x-content-sha256"));
    let body: PromptJson = serde_json::from_slice(&sent[0].body).expect("the body is JSON");
    let PromptJson::Object(fields) = body else {
        panic!("expected a JSON object body");
    };
    assert_eq!(
        fields.get("compartmentId"),
        Some(&PromptJson::Text(String::from(
            "ocid1.compartment.oc1..aaaa"
        )))
    );
    assert!(fields.contains_key("inputs"));
}

#[tokio::test]
async fn snowflake_reply_transform_keeps_the_source_model() {
    let reply = transform_embedding_response(
        "openai-gpt-4.1",
        r#"{"object":"list","data":[{"object":"embedding","embedding":[[0.1]],"index":0}],"model":"openai-gpt-4.1","usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )
    .expect("the snowflake reply is read");
    assert_eq!(reply.hidden_model, "openai-gpt-4.1");
}

fn extra_headers() -> BTreeMap<String, String> {
    BTreeMap::from([(String::from("x-oracle-extra"), String::from("leaked"))])
}

#[tokio::test]
async fn databricks_embedding_ignores_the_caller_headers() {
    let mut fixture = Fixture::new();
    fixture.environment = environment(&[
        (
            "DATABRICKS_API_BASE",
            "https://dbc.example.com/serving-endpoints",
        ),
        ("DATABRICKS_API_KEY", "db-key"),
    ]);
    fixture.headers = Some(extra_headers());
    let transport = Scripted::with_replies(vec![ok(databricks_embedding_reply())]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    embedding_dispatch(&call, EmbeddingConfig::Databricks, &context)
        .await
        .expect("the databricks embedding answers");
    let sent = transport.sent();
    assert!(!sent[0].headers.contains_key("x-oracle-extra"));
}

#[tokio::test]
async fn snowflake_embedding_ignores_the_caller_headers() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("openai-gpt-4.1");
    fixture.environment = environment(&[
        ("SNOWFLAKE_JWT", "pat/abc"),
        ("SNOWFLAKE_ACCOUNT_ID", "acct1"),
    ]);
    fixture.headers = Some(extra_headers());
    let transport = Scripted::with_replies(vec![ok(
        r#"{"object":"list","data":[{"object":"embedding","embedding":[[0.1,0.2]],"index":0}],"model":"openai-gpt-4.1","usage":{"prompt_tokens":1,"total_tokens":1}}"#,
    )]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    embedding_dispatch(&call, EmbeddingConfig::Snowflake, &context)
        .await
        .expect("the snowflake embedding answers");
    let sent = transport.sent();
    assert!(!sent[0].headers.contains_key("x-oracle-extra"));
}

#[tokio::test]
async fn oci_embedding_uses_the_global_api_base() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("cohere.embed-english-v3.0");
    fixture.environment = environment(&[
        ("OCI_USER", "ocid1.user.oc1..bbbb"),
        ("OCI_FINGERPRINT", "aa:bb:cc"),
        ("OCI_TENANCY", "ocid1.tenancy.oc1..cccc"),
        ("OCI_KEY", TEST_KEY),
        ("OCI_COMPARTMENT_ID", "ocid1.compartment.oc1..aaaa"),
    ]);
    let transport = Scripted::with_replies(vec![ok(
        r#"{"embeddings":[[0.1,0.2]],"model_id":"cohere.embed-english-v3.0","model_version":"1.0"}"#,
    )]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let mut globals = OwnedGlobals::empty();
    globals.api_base = Some(String::from("https://global.example.com"));
    let call = fixture.embedding_call();
    let context = embedding_context(&fixture, globals.borrowed(), &transport, &tokens, &clock);
    embedding_dispatch(&call, EmbeddingConfig::Oci, &context)
        .await
        .expect("the oci embedding answers");
    let sent = transport.sent();
    assert_eq!(
        sent[0].url,
        "https://global.example.com/20231130/actions/embedText"
    );
}

#[tokio::test]
async fn snowflake_embedding_keeps_the_error_headers() {
    let mut fixture = Fixture::new();
    fixture.model = String::from("openai-gpt-4.1");
    fixture.environment = environment(&[
        ("SNOWFLAKE_JWT", "pat/abc"),
        ("SNOWFLAKE_ACCOUNT_ID", "acct1"),
    ]);
    let transport = Scripted::with_replies(vec![status_with_headers(
        429,
        "slow down",
        "retry-after",
        "3",
    )]);
    let tokens = NoTokens;
    let clock = FixedClock;
    let call = fixture.embedding_call();
    let context = embedding_context(
        &fixture,
        DispatchGlobals::default(),
        &transport,
        &tokens,
        &clock,
    );
    match embedding_dispatch(&call, EmbeddingConfig::Snowflake, &context).await {
        Err(DispatchError::SnowflakeException(exception)) => {
            assert_eq!(exception.status_code, 429);
            assert_eq!(
                exception.headers.get("retry-after").map(String::as_str),
                Some("3")
            );
        }
        other => panic!("expected the snowflake exception, got {other:?}"),
    }
}

fn status_with_headers(
    code: u16,
    body: &str,
    name: &'static str,
    value: &'static str,
) -> UpstreamResponse {
    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static(name),
        HeaderValue::from_static(value),
    );
    UpstreamResponse {
        status: code,
        headers,
        body: body.as_bytes().to_vec(),
    }
}
