#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_models::decisions::DecisionsJson;
use betterlite_models::decisions::ir::{DecisionInput, DecisionRequest};
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::decisions::transformation::provider_config::DecisionsProviderConfig as _;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_openai::perplexity_decisions::PerplexityDecisionsConfig;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_openai::search::provider::SearchQuery;
use betterlite_prompts::arguments::json::{Json, Object};
use secrecy::SecretString;

use crate::hosted_platform_dispatch::call::{EmbeddingCall, ResourceCall};
use crate::hosted_platform_dispatch::context::DispatchSettings;
use crate::hosted_platform_dispatch::embedding::dispatch;
use crate::hosted_platform_dispatch::error::DispatchError;
use crate::hosted_platform_dispatch::perplexity;

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn body(bytes: &[u8]) -> Json {
    serde_json::from_slice(bytes).unwrap()
}

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn embedding_call() -> EmbeddingCall {
    EmbeddingCall {
        model: String::from("pplx-embed-v1-0.6b"),
        input: EmbeddingInput::Text(String::from("hello")),
        optional_params: object(r#"{"encoding_format": "float"}"#),
        litellm_params: object(r#"{"ssl_verify": false}"#),
        api_key: Some(SecretString::from(String::from("sk-passed"))),
        api_base: Some(String::from("https://mock.example")),
        headers: Some(BTreeMap::from([(
            String::from("X-Caller"),
            String::from("c1"),
        )])),
        timeout: Duration::from_secs(1),
        aembedding: false,
    }
}

fn resource_call<'call>(
    model: &'call str,
    optional_params: &'call Object,
    headers: &'call BTreeMap<String, String>,
    api_key: Option<&'call SecretString>,
) -> ResourceCall<'call> {
    ResourceCall {
        model,
        api_base: Some("https://mock.example"),
        api_key,
        headers: Some(headers),
        optional_params,
        litellm_params: optional_params,
    }
}

#[test]
fn reaches_config_embedding_through_the_perplexity_arm() {
    let call = embedding_call();
    let arm = dispatch(ProviderId::Perplexity, &call, DispatchSettings::default())
        .expect("perplexity has an arm of its own")
        .unwrap();
    assert!(arm.litellm_params.is_empty());
    assert_eq!(arm.headers, None);
    assert_eq!(arm.api_base.as_deref(), Some("https://mock.example"));
    assert_eq!(arm.model, call.model);
    let request = perplexity::embedding_request(&arm, &environment(&[])).unwrap();
    assert_eq!(request.url, "https://mock.example/v1/embeddings");
    assert_eq!(
        request.headers,
        BTreeMap::from([
            (
                String::from("Authorization"),
                String::from("Bearer sk-passed")
            ),
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
        ])
    );
    assert_eq!(
        body(&request.body),
        json(r#"{"model":"pplx-embed-v1-0.6b","input":"hello","encoding_format":"float"}"#)
    );
}

#[test]
fn reaches_config_responses_with_a_preset_body() {
    let params = object(r#"{"max_output_tokens": 20}"#);
    let headers = BTreeMap::from([(String::from("X-Caller"), String::from("c1"))]);
    let call = resource_call("preset/pro-search", &params, &headers, None);
    let input: ResponsesInput =
        serde_json::from_str(r#"[{"role":"user","content":"hi"}]"#).unwrap();
    let request = perplexity::responses(
        &call,
        &input,
        &environment(&[("PERPLEXITY_API_KEY", "sk-env")]),
    )
    .unwrap();
    assert_eq!(request.url, "https://mock.example/v1/responses");
    assert_eq!(
        request.headers,
        BTreeMap::from([
            (String::from("Authorization"), String::from("Bearer sk-env")),
            (String::from("X-Caller"), String::from("c1")),
        ])
    );
    assert_eq!(
        body(&request.body),
        json(
            r#"{"preset":"pro-search","input":[{"role":"user","content":"hi","type":"message"}],"max_output_tokens":20}"#
        )
    );
}

#[test]
fn reaches_config_responses_and_reports_a_bad_tools_value() {
    let params = object(r#"{"tools": "not a list"}"#);
    let headers = BTreeMap::new();
    let call = resource_call("sonar", &params, &headers, None);
    let input: ResponsesInput = serde_json::from_str(r#""hi""#).unwrap();
    let outcome = perplexity::responses(&call, &input, &environment(&[]));
    assert!(matches!(
        outcome,
        Err(DispatchError::PerplexityResponses(_))
    ));
}

#[test]
fn reaches_config_search_with_the_four_options() {
    let params = object(r#"{"max_results": 3, "search_recency_filter": "week"}"#);
    let headers = BTreeMap::new();
    let key = SecretString::from(String::from("sk-call"));
    let call = resource_call("search", &params, &headers, Some(&key));
    let query = SearchQuery::Single(String::from("rust"));
    let request = perplexity::search(&call, &query, &environment(&[])).unwrap();
    assert_eq!(request.url, "https://mock.example/search");
    assert_eq!(
        request.headers,
        BTreeMap::from([
            (
                String::from("Authorization"),
                String::from("Bearer sk-call")
            ),
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
        ])
    );
    assert_eq!(
        body(&request.body),
        json(r#"{"query":"rust","max_results":3}"#)
    );
}

#[test]
fn reaches_config_search_and_names_the_missing_key() {
    let params = Object::new();
    let headers = BTreeMap::new();
    let call = resource_call("search", &params, &headers, None);
    let query = SearchQuery::Single(String::from("rust"));
    let outcome = perplexity::search(
        &call,
        &query,
        &environment(&[("PERPLEXITY_API_KEY", "sk-other")]),
    );
    let Err(DispatchError::PerplexitySearch(error)) = outcome else {
        panic!("the search key is required");
    };
    assert_eq!(
        error.to_string(),
        "PERPLEXITYAI_API_KEY is not set. Set `PERPLEXITYAI_API_KEY` environment variable."
    );
}

#[test]
fn reaches_config_decisions_with_its_path_and_variables() {
    let params = Object::new();
    let headers = BTreeMap::from([(String::from("X-Caller"), String::from("c1"))]);
    let mut call = resource_call("pplx-decider", &params, &headers, None);
    call.api_base = Some("https://mock.example/v1/");
    let request = DecisionRequest {
        input: DecisionInput::State(DecisionsJson::Text(String::from("ready"))),
        questions: PerplexityDecisionsConfig.health_check_questions(),
        safety_identifier: None,
    };
    let built = perplexity::decisions(
        &call,
        &request,
        &environment(&[
            ("PERPLEXITYAI_API_KEY", ""),
            ("PERPLEXITY_API_KEY", "sk-second"),
        ]),
    )
    .unwrap();
    assert_eq!(built.url, "https://mock.example/v1/decisions");
    assert_eq!(
        built.headers,
        BTreeMap::from([
            (
                String::from("Authorization"),
                String::from("Bearer sk-second")
            ),
            (
                String::from("Content-Type"),
                String::from("application/json")
            ),
            (String::from("X-Caller"), String::from("c1")),
        ])
    );
    assert!(!built.body.is_empty());
}

#[test]
fn reaches_config_decisions_from_the_base_variable_and_the_default() {
    let params = Object::new();
    let headers = BTreeMap::new();
    let mut call = resource_call("pplx-decider", &params, &headers, None);
    let request = DecisionRequest {
        input: DecisionInput::State(DecisionsJson::Text(String::from("ready"))),
        questions: PerplexityDecisionsConfig.health_check_questions(),
        safety_identifier: None,
    };
    call.api_base = Some("");
    let from_variable = perplexity::decisions(
        &call,
        &request,
        &environment(&[("PERPLEXITY_API_BASE", "https://env.example")]),
    )
    .unwrap();
    assert_eq!(from_variable.url, "https://env.example/v1/decisions");
    assert!(!from_variable.headers.contains_key("Authorization"));
    call.api_base = None;
    let defaulted = perplexity::decisions(&call, &request, &environment(&[])).unwrap();
    assert_eq!(defaulted.url, "https://api.perplexity.ai/v1/decisions");
}
