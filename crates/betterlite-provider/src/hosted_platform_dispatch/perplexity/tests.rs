#![cfg(test)]

use std::fmt::Debug;
use std::time::Duration;

use betterlite_models::identity::provider_id::ProviderId;
use betterlite_openai::embedding::EmbeddingInput;
use betterlite_prompts::arguments::json::Object;

use crate::hosted_platform_dispatch::call::{EmbeddingCall, ResourceCall};
use crate::hosted_platform_dispatch::context::DispatchSettings;
use crate::hosted_platform_dispatch::embedding::dispatch;
use crate::hosted_platform_dispatch::error::DispatchError;
use crate::hosted_platform_dispatch::perplexity;

fn assert_not_built<T>(outcome: Result<T, DispatchError>, operation: &str)
where
    T: Debug,
{
    let error = outcome.expect_err("the operation must answer an error");
    assert_eq!(
        error.to_string(),
        format!("perplexity {operation} support is not built yet")
    );
    assert!(matches!(
        error,
        DispatchError::NotBuilt {
            provider: "perplexity",
            operation: built
        } if built == operation
    ));
}

fn embedding_call() -> EmbeddingCall {
    EmbeddingCall {
        model: String::from("pplx-embed-v1-0.6b"),
        input: EmbeddingInput::Text(String::from("hello")),
        optional_params: Object::new(),
        litellm_params: Object::new(),
        api_key: None,
        api_base: None,
        headers: None,
        timeout: Duration::from_secs(1),
        aembedding: false,
    }
}

#[test]
fn unbuilt_perplexity_embedding() {
    let call = embedding_call();
    let arm = dispatch(ProviderId::Perplexity, &call, DispatchSettings::default())
        .expect("perplexity has an arm of its own");
    assert_not_built(arm, "embedding");
}

#[test]
fn unbuilt_perplexity_responses() {
    let params = Object::new();
    let call = ResourceCall {
        model: "preset/fast-search",
        api_base: None,
        api_key: None,
        headers: None,
        optional_params: &params,
        litellm_params: &params,
    };
    assert_not_built(perplexity::responses(&call), "responses");
}

#[test]
fn unbuilt_perplexity_search() {
    let params = Object::new();
    let call = ResourceCall {
        model: "search",
        api_base: None,
        api_key: None,
        headers: None,
        optional_params: &params,
        litellm_params: &params,
    };
    assert_not_built(perplexity::search(&call), "search");
}

#[test]
fn unbuilt_perplexity_decisions() {
    let params = Object::new();
    let call = ResourceCall {
        model: "pplx-decider",
        api_base: None,
        api_key: None,
        headers: None,
        optional_params: &params,
        litellm_params: &params,
    };
    assert_not_built(perplexity::decisions(&call), "decisions");
}
