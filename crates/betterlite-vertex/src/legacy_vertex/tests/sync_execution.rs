use std::sync::{Arc, Mutex};

use betterlite_cache::evicted_client_closer::{Clock, EvictedClientCloser};
use betterlite_cache::llm_client_cache::LlmClientCache;
use betterlite_config::sections::vertex_credential_sources::credential_source::ConfiguredCredentials;
use betterlite_models::decisions::Json;
use betterlite_models::provider_model_sets::ProviderModelSet;
use futures_util::StreamExt as _;

use crate::legacy_prediction::error::PredictionError;
use crate::legacy_prediction::projection::LegacyMessage;
use crate::legacy_vertex::LegacyCompletion;
use crate::legacy_vertex::cache::ClientCache;
use crate::legacy_vertex::complete;
use crate::legacy_vertex::error::LegacyVertexError;
use crate::token_cache::error::CredentialCacheError;

use super::support::{
    Call, CountingEncoding, FixtureCache, FixtureClient, PlanFixture, access, chunk, content,
    fake_stream_chunk, json, messages, options, predictions_reply, predictions_reply_with_usage,
    request, services, text_reply, token_lookup,
};

#[tokio::test]
async fn cache_read_without_write() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("chat-bison", &contents, options("{}"), false);
    let client = FixtureClient::with_text(text_reply("hi there"));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    assert!(matches!(completion, LegacyCompletion::Response(_)));
    assert_eq!(
        cache.keys(),
        vec![String::from("chat-bison-proj-us-central1")]
    );
}

#[test]
fn cache_adapter_reads_the_entry() {
    let clock: Clock = Arc::new(|| 1_000.0);
    let closer = Arc::new(EvictedClientCloser::new(Arc::clone(&clock)));
    let cache = LlmClientCache::<FixtureClient>::new(10, 60.0, clock, closer).expect("a cache");
    cache
        .set_cache(
            "chat-bison-proj-us-central1",
            Arc::new(FixtureClient::default()),
            None,
            false,
        )
        .expect("a stored client");
    let found = cache
        .cached("chat-bison-proj-us-central1")
        .expect("the cache read");
    assert!(found.is_some());
    assert!(cache.cached("missing").expect("the cache read").is_none());
}

#[tokio::test]
async fn sync_error_mapping() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages("[]");
    let call = request("chat-bison", &contents, options("{}"), false);
    let client = FixtureClient::with_text_failure(PredictionError::MissingPrediction);
    let cache = FixtureCache::empty();
    let error = match complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    {
        Err(error) => error,
        Ok(_completion) => panic!("the call fails"),
    };
    assert!(matches!(error, LegacyVertexError::Prediction(_)));
    assert_eq!(error.status(), 500);
}

#[tokio::test]
async fn delegated_stream_keeps_status() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages("[]");
    let call = request(
        "chat-bison",
        &contents,
        options(r#"{"stream": true}"#),
        true,
    );
    let client = FixtureClient::with_stream_failure(PredictionError::Http {
        status: 500,
        message: String::from("429 Quota exceeded for vertex_ai"),
    });
    let cache = FixtureCache::empty();
    let error = match complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    {
        Err(error) => error,
        Ok(_completion) => panic!("the call fails"),
    };
    assert!(matches!(error, LegacyVertexError::Delegated(_)));
    assert_eq!(error.status(), 429);
}

#[tokio::test]
async fn cached_client_serves_chat() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("chat-bison", &contents, options("{}"), false);
    let cached = Arc::new(FixtureClient::with_text(text_reply("cached answer")));
    let injected = FixtureClient::with_text(text_reply("injected answer"));
    let cache = FixtureCache::with_client(Arc::clone(&cached));
    let completion = complete(
        &cache,
        &injected,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "cached answer");
    assert!(injected.calls().is_empty());
    assert_eq!(cached.calls().len(), 1);
}

#[tokio::test]
async fn custom_uses_injected_client() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("custom-model", &contents, options("{}"), false);
    let cached = Arc::new(FixtureClient::with_endpoint(predictions_reply(&["cached"])));
    let injected = FixtureClient::with_endpoint(predictions_reply(&["injected"]));
    let cache = FixtureCache::with_client(Arc::clone(&cached));
    let completion = complete(
        &cache,
        &injected,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "injected");
    assert!(cached.calls().is_empty());
}

#[tokio::test]
async fn sync_ignores_server_usage() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("custom-model", &contents, options("{}"), false);
    let reply = predictions_reply_with_usage(&["first", "second"], 100, 200, 300);
    let client = FixtureClient::with_endpoint(reply);
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "first");
    assert_eq!(result.usage.prompt_tokens, 5);
    assert_eq!(result.usage.completion_tokens, 5);
    assert_eq!(result.usage.total_tokens, 10);
}

#[tokio::test]
async fn empty_predictions_fail() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages("[]");
    let call = request("custom-model", &contents, options("{}"), false);
    let client = FixtureClient::with_endpoint(predictions_reply(&[]));
    let cache = FixtureCache::empty();
    let error = match complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    {
        Err(error) => error,
        Ok(_completion) => panic!("the call fails"),
    };
    assert!(matches!(
        error,
        LegacyVertexError::Prediction(PredictionError::MissingPrediction)
    ));
    assert_eq!(error.status(), 500);
}

#[tokio::test]
async fn sync_scope_uses_resolved_project() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let mut fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    fixture.project = None;
    let contents = messages("[]");
    let call = request("chat-bison", &contents, options("{}"), false);
    let client = FixtureClient::with_text(text_reply("ok"));
    let cache = FixtureCache::empty();
    complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let calls = client.calls();
    let Call::Message(request) = &calls[0] else {
        panic!("a message call");
    };
    assert_eq!(request.project, "resolved-project");
}

#[tokio::test]
async fn private_builds_endpoint() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request(
        "private",
        &contents,
        options(r#"{"model_id": "123"}"#),
        false,
    );
    let client = FixtureClient::with_endpoint(predictions_reply(&["answer"]));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(result.model, "123");
    assert_eq!(content(&result.message), "answer");
    let calls = client.calls();
    assert_eq!(calls.len(), 1);
    let Call::Endpoint(request) = &calls[0] else {
        panic!("an endpoint call");
    };
    assert_eq!(
        request.endpoint,
        "projects/proj/locations/us-central1/endpoints/123"
    );
    assert_eq!(request.instances[0], json(r#"{"prompt":"hello"}"#));
}

#[tokio::test]
async fn async_private_endpoint() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let mut fixture = PlanFixture::new(&[]);
    fixture.project = None;
    let contents = messages("[]");
    let call = request(
        "private",
        &contents,
        options(r#"{"model_id": "777"}"#),
        true,
    );
    let client = FixtureClient::with_endpoint(predictions_reply(&["async answer"]));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(result.model, "777");
    let calls = client.calls();
    let Call::Endpoint(request) = &calls[0] else {
        panic!("an endpoint call");
    };
    assert_eq!(
        request.endpoint,
        "projects/resolved-project/locations/us-central1/endpoints/777"
    );
}

#[tokio::test]
async fn custom_stream_returns_words() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages("[]");
    let call = request(
        "custom-model",
        &contents,
        options(r#"{"stream": true}"#),
        false,
    );
    let client = FixtureClient::with_endpoint(predictions_reply(&["alpha beta"]));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Words(words) = completion else {
        panic!("a word stream");
    };
    let collected: Vec<String> = words.collect();
    assert_eq!(collected, vec!["alpha", "beta"]);
    let calls = client.calls();
    let Call::Endpoint(request) = &calls[0] else {
        panic!("an endpoint call");
    };
    assert_eq!(request.instances[0], json(r#"{"stream":true,"prompt":""}"#));
}

#[tokio::test]
async fn sync_stream_returns_sdk() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages("[]");
    let call = request(
        "chat-bison",
        &contents,
        options(r#"{"stream": true}"#),
        false,
    );
    let client = FixtureClient::with_stream(vec![chunk("one"), chunk("two")]);
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Prediction(stream) = completion else {
        panic!("a prediction stream");
    };
    let texts: Vec<String> = stream
        .map(|chunk| chunk.expect("a streamed chunk").text)
        .collect()
        .await;
    assert_eq!(texts, vec!["one", "two"]);
    let calls = client.calls();
    let Call::MessageStream(request) = &calls[0] else {
        panic!("a streaming message call");
    };
    assert_eq!(request.options.get("stream"), None);
}

#[tokio::test]
async fn sync_chat_projects_usage() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("chat-bison", &contents, options("{}"), false);
    let client = FixtureClient::with_text(text_reply("hi there"));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "hi there");
    assert_eq!(result.model, "chat-bison");
    assert_eq!(result.created, 1_700_000_000);
    assert!(result.finish_reason.is_none());
    assert_eq!(result.usage.prompt_tokens, 5);
    assert_eq!(result.usage.completion_tokens, 8);
    assert_eq!(result.usage.total_tokens, 13);
}

#[tokio::test]
async fn safety_not_forwarded() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages("[]");
    let call = request(
        "chat-bison",
        &contents,
        options(r#"{"temperature": 0.4, "safety_settings": [{"category": "x"}]}"#),
        false,
    );
    let client = FixtureClient::with_text(text_reply("ok"));
    let cache = FixtureCache::empty();
    complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let calls = client.calls();
    let Call::Message(request) = &calls[0] else {
        panic!("a message call");
    };
    assert_eq!(request.options.get("safety_settings"), None);
    let temperature = request.options.get("temperature").expect("the option");
    assert_eq!(serde_json::to_string(temperature).expect("a number"), "0.4");
}

#[tokio::test]
async fn empty_reply_keeps_content() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("chat-bison", &contents, options("{}"), false);
    let client = FixtureClient::with_text(text_reply(""));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(result.message, LegacyMessage::Unchanged);
    assert_eq!(result.usage.prompt_tokens, 5);
    assert_eq!(result.usage.completion_tokens, 0);
    assert_eq!(result.usage.total_tokens, 5);
}

#[tokio::test]
async fn fake_stream_renders_response() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexCodeText, "code-bison")]);
    let client = FixtureClient::with_text(text_reply("a b"));
    let Json::Object(mut response) = fake_stream_chunk(&fixture, &projection, &client).await else {
        panic!("a response object");
    };
    let Some(Json::Text(id)) = response.shift_remove("id") else {
        panic!("a response id");
    };
    assert!(id.starts_with("chatcmpl-"));
    let expected = json(
        r#"{"object":"chat.completion","created":1700000000,"model":"code-bison","system_fingerprint":null,"choices":[{"finish_reason":"stop","index":0,"message":{"content":"a b","role":"assistant","tool_calls":null,"function_call":null,"provider_specific_fields":null}}],"usage":{"completion_tokens":3,"prompt_tokens":5,"total_tokens":8,"completion_tokens_details":null,"prompt_tokens_details":null}}"#,
    );
    assert_eq!(Json::Object(response), expected);
}

#[tokio::test]
async fn fake_stream_nulls_empty() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexCodeText, "code-bison")]);
    let client = FixtureClient::with_text(text_reply(""));
    let Json::Object(mut response) = fake_stream_chunk(&fixture, &projection, &client).await else {
        panic!("a response object");
    };
    let Some(Json::Text(id)) = response.shift_remove("id") else {
        panic!("a response id");
    };
    assert!(id.starts_with("chatcmpl-"));
    let expected = json(
        r#"{"object":"chat.completion","created":1700000000,"model":"code-bison","system_fingerprint":null,"choices":[{"finish_reason":"stop","index":0,"message":{"content":null,"role":"assistant","tool_calls":null,"function_call":null,"provider_specific_fields":null}}],"usage":{"completion_tokens":0,"prompt_tokens":5,"total_tokens":5,"completion_tokens_details":null,"prompt_tokens_details":null}}"#,
    );
    assert_eq!(Json::Object(response), expected);
}

#[tokio::test]
async fn vision_returns_none() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let mut fixture = PlanFixture::new(&[(ProviderModelSet::VertexVision, "vision-model")]);
    fixture.location = None;
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("vision-model", &contents, options("{}"), false);
    let client = FixtureClient::default();
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "None");
    assert_eq!(result.model, "vision-model");
    assert_eq!(result.usage.prompt_tokens, 5);
    assert_eq!(result.usage.completion_tokens, 4);
    assert_eq!(result.usage.total_tokens, 9);
    assert!(client.calls().is_empty());
}

#[tokio::test]
async fn missing_location_fails() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let mut fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    fixture.location = None;
    let contents = messages("[]");
    let call = request("chat-bison", &contents, options("{}"), false);
    let client = FixtureClient::default();
    let cache = FixtureCache::empty();
    let error = match complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    {
        Err(error) => error,
        Ok(_completion) => panic!("the call fails"),
    };
    assert!(matches!(error, LegacyVertexError::MissingLocation));
    assert_eq!(error.status(), 500);
}

#[tokio::test]
async fn async_chat_delegates() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("chat-bison", &contents, options("{}"), true);
    let client = FixtureClient::with_text(text_reply("delegated"));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "delegated");
    assert_eq!(result.usage.prompt_tokens, 1);
    assert_eq!(result.usage.completion_tokens, 2);
    assert_eq!(result.usage.total_tokens, 3);
    let calls = client.calls();
    assert!(matches!(&calls[0], Call::Message(_)));
}

#[tokio::test]
async fn async_stream_delegates() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages("[]");
    let call = request(
        "chat-bison",
        &contents,
        options(r#"{"stream": true}"#),
        true,
    );
    let client = FixtureClient::with_stream(vec![chunk("one")]);
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Chunks(chunks) = completion else {
        panic!("the delegated chunks");
    };
    let texts: Vec<String> = chunks
        .map(|chunk| chunk.expect("a streamed chunk").text)
        .collect()
        .await;
    assert_eq!(texts, vec!["one"]);
    let calls = client.calls();
    assert!(matches!(&calls[0], Call::MessageStream(_)));
}

#[tokio::test]
async fn project_reaches_credentials() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexVision, "vision-model")]);
    let contents = messages("[]");
    let call = request("vision-model", &contents, options("{}"), false);
    let client = FixtureClient::default();
    let cache = FixtureCache::empty();
    let seen: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let shared = Arc::clone(&seen);
    let make = move |_credentials: Option<&ConfiguredCredentials>, project: Option<&str>| {
        let record = Arc::clone(&shared);
        let project = project.map(str::to_owned);
        async move {
            *record.lock().expect("the recorded project") = project;
            Ok::<_, CredentialCacheError>(access())
        }
    };
    complete(&cache, &client, call, fixture.deps(&projection), make)
        .await
        .expect("a completion");
    assert_eq!(
        *seen.lock().expect("the recorded project"),
        Some(String::from("proj"))
    );
}

#[tokio::test]
async fn text_uses_predict() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexText, "text-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("text-bison", &contents, options("{}"), false);
    let client = FixtureClient::with_text(text_reply("answer"));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "answer");
    let calls = client.calls();
    assert!(matches!(&calls[0], Call::Text(_)));
}

#[tokio::test]
async fn text_stream_returns_sdk() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexText, "text-bison")]);
    let contents = messages("[]");
    let call = request(
        "text-bison",
        &contents,
        options(r#"{"stream": true}"#),
        false,
    );
    let client = FixtureClient::with_stream(vec![chunk("word")]);
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Prediction(stream) = completion else {
        panic!("a prediction stream");
    };
    let texts: Vec<String> = stream
        .map(|chunk| chunk.expect("a streamed chunk").text)
        .collect()
        .await;
    assert_eq!(texts, vec!["word"]);
    let calls = client.calls();
    let Call::TextStream(request) = &calls[0] else {
        panic!("a streaming text call");
    };
    assert_eq!(request.options.get("stream"), None);
}

#[tokio::test]
async fn code_text_without_stream_returns_response() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexCodeText, "code-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("code-bison", &contents, options("{}"), false);
    let client = FixtureClient::with_text(text_reply("plain"));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "plain");
}

#[tokio::test]
async fn async_text_delegates() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexText, "text-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("text-bison", &contents, options("{}"), true);
    let client = FixtureClient::with_text(text_reply("delegated text"));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "delegated text");
    assert_eq!(result.usage.prompt_tokens, 1);
    assert_eq!(result.usage.completion_tokens, 2);
    assert_eq!(result.usage.total_tokens, 3);
    let calls = client.calls();
    assert!(matches!(&calls[0], Call::Text(_)));
}

#[tokio::test]
async fn async_custom_endpoint() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("custom-model", &contents, options("{}"), true);
    let client = FixtureClient::with_endpoint(predictions_reply(&["async custom"]));
    let cache = FixtureCache::empty();
    let completion = complete(
        &cache,
        &client,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "async custom");
    let calls = client.calls();
    let Call::Endpoint(request) = &calls[0] else {
        panic!("an endpoint call");
    };
    assert_eq!(
        request.endpoint,
        "projects/proj/locations/us-central1/endpoints/custom-model"
    );
}

#[tokio::test]
async fn async_cached_client_serves_chat() {
    let encoder = CountingEncoding;
    let projection = services(&encoder);
    let fixture = PlanFixture::new(&[(ProviderModelSet::VertexChat, "chat-bison")]);
    let contents = messages(r#"[{"role": "user", "content": "hello"}]"#);
    let call = request("chat-bison", &contents, options("{}"), true);
    let cached = Arc::new(FixtureClient::with_text(text_reply("cached async")));
    let injected = FixtureClient::with_text(text_reply("injected async"));
    let cache = FixtureCache::with_client(Arc::clone(&cached));
    let completion = complete(
        &cache,
        &injected,
        call,
        fixture.deps(&projection),
        token_lookup,
    )
    .await
    .expect("a completion");
    let LegacyCompletion::Response(result) = completion else {
        panic!("a response");
    };
    assert_eq!(content(&result.message), "cached async");
    assert!(injected.calls().is_empty());
    assert_eq!(cached.calls().len(), 1);
}
