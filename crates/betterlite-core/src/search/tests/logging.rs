#![cfg(test)]

use std::time::Duration;

use betterlite_http_fixture::reply::reply;
use betterlite_http_fixture::server::serve;
use betterlite_models::decisions::Json;
use betterlite_openai::openai_base_config::Object;
use indexmap::IndexMap;

use super::SearchOptions;
use super::support::{ANSWER, Harness, Recorder, Registry, json, object, request};
use crate::api::search_providers::parallel_ai::request::PARALLEL_AI_USAGE_PARAM;

fn kwargs(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the kwargs fixture parses")
}

#[tokio::test]
async fn logging_records_the_search_call() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let base = format!("http://{}", served.address);
    let registry = Registry::new(Recorder::default());
    let mut search = request(r#""rust""#, &base);
    search.options = SearchOptions {
        max_results: Some(2_i64),
        search_domain_filter: None,
        max_tokens_per_page: None,
        country: Some("DE".to_owned()),
    };
    search.kwargs =
        kwargs(r#"{"litellm_call_id":"call-77","metadata":{"team":"search"},"safesearch":"off"}"#);
    let mut harness = Harness::new();
    harness.search(&search, &registry).await.unwrap();
    let state = &harness.state;
    assert_eq!(state.model, "brave/search");
    assert_eq!(state.custom_llm_provider.as_deref(), Some("brave"));
    assert_eq!(
        Json::Object(state.optional_params.clone()),
        json(r#"{"max_results":2,"country":"DE","safesearch":"off"}"#)
    );
    assert_eq!(
        state.litellm_params.get("litellm_call_id"),
        Some(&Json::Text("call-77".to_owned()))
    );
    assert_eq!(
        state.litellm_params.get("api_base"),
        Some(&Json::Text(format!("{base}/v1/search")))
    );
    assert_eq!(
        state.litellm_params.get("metadata"),
        Some(&json(r#"{"team":"search"}"#))
    );
    served.finish().expect("the fixture answers");
}

#[tokio::test]
async fn logging_writes_a_null_call_id_when_kwargs_carry_none() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let registry = Registry::new(Recorder::default());
    let mut harness = Harness::new();
    harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap();
    assert_eq!(
        harness.state.litellm_params.get("litellm_call_id"),
        Some(&Json::Null)
    );
    assert!(harness.state.optional_params.is_empty());
    served.finish().expect("the fixture answers");
}

#[tokio::test]
async fn logging_copies_the_parallel_usage_into_the_optional_params() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let hidden = object(r#"{"_parallel_ai_usage":[{"name":"sku_search","count":1}],"headers":{}}"#);
    let registry = Registry::new(Recorder::new(false, hidden));
    let mut harness = Harness::new();
    let response = harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap();
    assert_eq!(
        harness.state.optional_params.get(PARALLEL_AI_USAGE_PARAM),
        Some(&json(r#"[{"name":"sku_search","count":1}]"#))
    );
    assert!(!harness.state.optional_params.contains_key("headers"));
    assert_eq!(response.hidden_params.len(), 2);
    served.finish().expect("the fixture answers");
}

#[tokio::test]
async fn logging_adds_no_usage_when_the_reply_has_none() {
    let served = serve(reply("200 OK", &[], ANSWER), Duration::ZERO).expect("the fixture serves");
    let registry = Registry::new(Recorder::new(false, Object::new()));
    let mut harness = Harness::new();
    harness
        .search(
            &request(r#""rust""#, &format!("http://{}", served.address)),
            &registry,
        )
        .await
        .unwrap();
    assert!(
        !harness
            .state
            .optional_params
            .contains_key(PARALLEL_AI_USAGE_PARAM)
    );
    served.finish().expect("the fixture answers");
}
