use std::time::SystemTime;

use betterlite_router::deployment_identity::spec::DeploymentSpec;
use betterlite_router::model_catalog_lookup::alias::AliasMap;
use rust_decimal::Decimal;

use crate::spend::estimates::decode::decode_spend_estimate;
use crate::spend::estimates::error::EstimateError;
use crate::spend::estimates::estimate::estimate;
use crate::spend::estimates::router::EstimatesState;

use super::support::{alias, configured_router, decimal, deployment, state};

const MESSAGES: &str = r#"[{"role":"user","content":"hi"}]"#;

async fn priced(state: &EstimatesState, body: &str) -> Result<Decimal, EstimateError> {
    let request = decode_spend_estimate(body.as_bytes()).unwrap();
    estimate(
        &state.cost,
        state.counter.as_ref(),
        state.router.as_deref(),
        &request,
        SystemTime::from(state.clock.now()),
    )
    .await
}

fn mini() -> DeploymentSpec {
    deployment("mini", r#"{"model":"openai/gpt-4o-mini"}"#)
}

#[tokio::test]
async fn prices_the_matched_deployments_underlying_model() {
    let router = configured_router(vec![mini()], AliasMap::new(), None, None);
    let cost = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.0000015"));
}

#[tokio::test]
async fn resolves_a_model_group_alias_before_searching() {
    let router = configured_router(vec![mini()], alias("quote-alias", "mini"), None, None);
    let cost = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"quote-alias","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.0000015"));
}

#[tokio::test]
async fn retains_the_last_matching_deployment() {
    let router = configured_router(
        vec![
            deployment("shared", r#"{"model":"not-a-model"}"#),
            deployment("shared", r#"{"model":"openai/gpt-4o-mini"}"#),
        ],
        AliasMap::new(),
        None,
        None,
    );
    let cost = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"shared","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.0000015"));
}

#[tokio::test]
async fn an_alias_without_a_match_does_not_fall_back_to_the_requested_name() {
    let router = configured_router(
        vec![deployment(
            "quote-alias",
            r#"{"model":"openai/gpt-4o-mini"}"#,
        )],
        alias("quote-alias", "missing-group"),
        None,
        None,
    );
    let error = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"quote-alias","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::ProviderMissing { .. }));
}

#[tokio::test]
async fn bills_the_deployments_token_overrides() {
    let router = configured_router(
        vec![deployment(
            "mini",
            r#"{"model":"openai/gpt-4o-mini","input_cost_per_token":0.001,"output_cost_per_token":0.002}"#,
        )],
        AliasMap::new(),
        None,
        None,
    );
    let cost = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.01"));
}

#[tokio::test]
async fn a_deployment_with_one_token_override_fails() {
    let router = configured_router(
        vec![deployment(
            "mini",
            r#"{"model":"openai/gpt-4o-mini","input_cost_per_token":0.001}"#,
        )],
        AliasMap::new(),
        None,
        None,
    );
    let error = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::Failure { .. }));
}

#[tokio::test]
async fn an_empty_deployment_model_fails_the_price() {
    let router = configured_router(
        vec![deployment("mini", r#"{"model":""}"#)],
        AliasMap::new(),
        None,
        None,
    );
    let error = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::ProviderMissing { .. }));
}

#[tokio::test]
async fn prices_an_empty_deployment_model_from_its_token_overrides() {
    let router = configured_router(
        vec![deployment(
            "mini",
            r#"{"model":"","input_cost_per_token":0.001,"output_cost_per_token":0.002}"#,
        )],
        AliasMap::new(),
        None,
        None,
    );
    let cost = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.01"));
}

#[tokio::test]
async fn a_deployment_without_a_model_fails_the_price() {
    let router = configured_router(vec![deployment("mini", "{}")], AliasMap::new(), None, None);
    let error = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::Failure { .. }));
}

#[tokio::test]
async fn a_response_token_count_beyond_a_signed_counter_fails() {
    let response = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
        "model":"gpt-4o-mini","choices":[],
        "usage":{"prompt_tokens":18446744073709551615,"completion_tokens":5,"total_tokens":20}}"#;
    let error = priced(
        &state(None),
        &format!(r#"{{"completion_response":{response}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::Failure { .. }));
}

#[tokio::test]
async fn a_deployment_with_a_non_numeric_override_fails() {
    let router = configured_router(
        vec![deployment(
            "mini",
            r#"{"model":"openai/gpt-4o-mini","input_cost_per_token":"cheap","output_cost_per_token":0.002}"#,
        )],
        AliasMap::new(),
        None,
        None,
    );
    let error = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::Failure { .. }));
}

#[tokio::test]
async fn ignores_pricing_fields_in_model_info() {
    let router = configured_router(
        vec![deployment(
            "mini",
            r#"{"model":"openai/gpt-4o-mini","model_info":{"input_cost_per_token":5}}"#,
        )],
        AliasMap::new(),
        None,
        None,
    );
    let cost = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.0000015"));
}

#[tokio::test]
async fn passes_an_unmatched_model_to_the_catalog() {
    let cost = priced(
        &state(None),
        &format!(r#"{{"model":"gpt-4o-mini","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.0000015"));
}

#[tokio::test]
async fn names_a_model_no_catalog_entry_maps() {
    let error = priced(
        &state(None),
        &format!(r#"{{"model":"openai/not-a-model","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::ModelNotMapped { .. }));
}

#[tokio::test]
async fn a_model_without_a_provider_fails_the_price() {
    let error = priced(
        &state(None),
        &format!(r#"{{"model":"not-a-model","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::ProviderMissing { .. }));
}

#[tokio::test]
async fn an_empty_request_model_fails_the_price() {
    let error = priced(
        &state(None),
        &format!(r#"{{"model":"","messages":{MESSAGES}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::ProviderMissing { .. }));
}

#[tokio::test]
async fn prices_a_response_only_request_from_its_usage() {
    let response = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
        "model":"gpt-4o-mini","choices":[],
        "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
    let cost = priced(
        &state(None),
        &format!(r#"{{"completion_response":{response}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.0000045"));
}

#[tokio::test]
async fn bills_a_responses_cached_tokens_at_the_cache_read_rate() {
    let response = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
        "model":"gpt-4o-mini","choices":[],
        "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15,
            "prompt_tokens_details":{"cached_tokens":10}}}"#;
    let cost = priced(
        &state(None),
        &format!(r#"{{"completion_response":{response}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.00000375"));
}

#[tokio::test]
async fn a_supplied_model_wins_over_a_supplied_response() {
    let response = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
        "model":"gpt-4o","choices":[],
        "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
    let router = configured_router(vec![mini()], AliasMap::new(), None, None);
    let cost = priced(
        &state(Some(router)),
        &format!(r#"{{"model":"mini","messages":{MESSAGES},"completion_response":{response}}}"#),
    )
    .await
    .unwrap();
    assert_eq!(cost, decimal("0.0000015"));
}

#[tokio::test]
async fn an_empty_message_list_is_still_a_supplied_messages_field() {
    let router = configured_router(vec![mini()], AliasMap::new(), None, None);
    let cost = priced(&state(Some(router)), r#"{"model":"mini","messages":[]}"#)
        .await
        .unwrap();
    assert_eq!(cost, Decimal::ZERO);
}

#[tokio::test]
async fn a_model_without_messages_is_a_bad_request() {
    let error = priced(&state(None), r#"{"model":"gpt-4o-mini"}"#)
        .await
        .unwrap_err();
    assert!(matches!(error, EstimateError::MessagesRequired));
}

#[tokio::test]
async fn a_model_without_messages_wins_over_a_response() {
    let response = r#"{"id":"chatcmpl-1","object":"chat.completion","created":1,
        "model":"gpt-4o-mini","choices":[],
        "usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#;
    let error = priced(
        &state(None),
        &format!(r#"{{"model":"gpt-4o-mini","completion_response":{response}}}"#),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, EstimateError::MessagesRequired));
}

#[tokio::test]
async fn neither_input_is_a_bad_request() {
    let error = priced(&state(None), "{}").await.unwrap_err();
    assert!(matches!(error, EstimateError::InputRequired));
}
