#![cfg(test)]

mod support;

use betterlite_tokenizer::engine::error::EngineError;
use betterlite_tokenizer::engine::selection::SelectionError;
use rstest::rstest;
use serde_json::json;

use super::error::EmbeddingInputError;
use super::{
    ConfiguredTokenLimits, EMBEDDING_METADATA_FLAG, MetadataMap, ModelListEntry,
    build_router_embedding_metadata, resolve_embedding_max_input_tokens, resolve_embedding_router,
    resolve_embedding_timeout, truncate_embedding_input,
};

const MODEL: &str = "text-embedding-3-small";

struct FakeRouter;

fn entry(model_name: Option<&str>) -> ModelListEntry {
    ModelListEntry {
        model_name: model_name.map(str::to_owned),
    }
}

#[rstest]
#[case::listing_deployment(true, Some(vec![Some(MODEL)]), true)]
#[case::no_router(false, Some(vec![Some(MODEL)]), false)]
#[case::no_model_list(true, None, false)]
#[case::empty_model_list(true, Some(Vec::new()), false)]
#[case::another_model_only(true, Some(vec![Some("other-embedding-model")]), false)]
#[case::nameless_entries_are_skipped(true, Some(vec![None, Some(MODEL)]), true)]
#[case::only_nameless_entries(true, Some(vec![None]), false)]
#[case::model_names_match_exactly(true, Some(vec![Some("Text-Embedding-3-Small")]), false)]
fn router_resolution_requires_a_listed_deployment(
    #[case] has_router: bool,
    #[case] model_names: Option<Vec<Option<&str>>>,
    #[case] expected: bool,
) {
    let router = FakeRouter;
    let model_list: Option<Vec<ModelListEntry>> = model_names
        .as_deref()
        .map(|names| names.iter().map(|name| entry(*name)).collect());
    let resolved =
        resolve_embedding_router(MODEL, has_router.then_some(&router), model_list.as_deref());
    assert_eq!(resolved.is_some(), expected);
}

#[test]
fn metadata_copies_the_caller_map_in_order_and_overwrites_the_flag_in_place() {
    let mut caller = MetadataMap::new();
    caller.insert("first".to_owned(), json!(1_i64));
    caller.insert(EMBEDDING_METADATA_FLAG.to_owned(), json!("old"));
    caller.insert("last".to_owned(), json!(true));
    let built = build_router_embedding_metadata(Some(&caller));
    let keys: Vec<&str> = built.keys().map(String::as_str).collect();
    assert_eq!(keys, ["first", EMBEDDING_METADATA_FLAG, "last"]);
    assert_eq!(built.get(EMBEDDING_METADATA_FLAG), Some(&json!(true)));
}

#[test]
fn metadata_leaves_the_caller_map_unchanged() {
    let mut caller = MetadataMap::new();
    caller.insert(EMBEDDING_METADATA_FLAG.to_owned(), json!("old"));
    let built = build_router_embedding_metadata(Some(&caller));
    assert_eq!(caller.get(EMBEDDING_METADATA_FLAG), Some(&json!("old")));
    assert_eq!(caller.len(), 1);
    assert_ne!(
        built.get(EMBEDDING_METADATA_FLAG),
        caller.get(EMBEDDING_METADATA_FLAG)
    );
}

#[test]
fn metadata_without_caller_metadata_holds_only_the_flag() {
    let built = build_router_embedding_metadata(None);
    assert_eq!(built.len(), 1);
    assert_eq!(built.get(EMBEDDING_METADATA_FLAG), Some(&json!(true)));
}

#[test]
fn metadata_appends_the_flag_after_a_new_caller_key() {
    let mut caller = MetadataMap::new();
    caller.insert("only".to_owned(), json!("value"));
    let built = build_router_embedding_metadata(Some(&caller));
    let keys: Vec<&str> = built.keys().map(String::as_str).collect();
    assert_eq!(keys, ["only", EMBEDDING_METADATA_FLAG]);
}

type DeploymentCase = Option<(&'static str, (Option<i64>, Option<i64>))>;

struct DeploymentLimits {
    model_name: String,
    limits: (Option<i64>, Option<i64>),
}

impl ConfiguredTokenLimits for DeploymentLimits {
    fn get_configured_token_limits(&self, model_name: &str) -> (Option<i64>, Option<i64>) {
        if model_name == self.model_name {
            self.limits
        } else {
            (None, None)
        }
    }
}

#[rstest]
#[case::configured_wins_over_the_deployment(Some(1000), Some((MODEL, (Some(500), Some(100)))), Some(1000))]
#[case::configured_zero_is_kept(Some(0), Some((MODEL, (Some(500), None))), Some(0))]
#[case::configured_negative_is_kept(Some(-3), Some((MODEL, (None, None))), Some(-3))]
#[case::deployment_limit_is_used(None, Some((MODEL, (Some(500), Some(100)))), Some(500))]
#[case::deployment_without_a_limit_gives_none(None, Some((MODEL, (None, Some(100)))), None)]
#[case::unknown_model_gives_none(None, Some(("other-embedding-model", (Some(500), None))), None)]
#[case::no_router_gives_none(None, None, None)]
fn max_tokens_resolution_prefers_the_configured_value(
    #[case] configured: Option<i64>,
    #[case] deployment: DeploymentCase,
    #[case] expected: Option<i64>,
) {
    let router = deployment.map(|(model_name, limits)| DeploymentLimits {
        model_name: model_name.to_owned(),
        limits,
    });
    let resolved = router.as_ref().map_or_else(
        || resolve_embedding_max_input_tokens(configured, MODEL, None),
        |limits| resolve_embedding_max_input_tokens(configured, MODEL, Some(limits)),
    );
    assert_eq!(resolved, expected);
}

#[rstest]
#[case::configured_wins(Some(2.5_f64), 5.0_f64, 2.5_f64)]
#[case::configured_zero_is_kept(Some(0.0_f64), 5.0_f64, 0.0_f64)]
#[case::configured_negative_is_kept(Some(-1.5_f64), 5.0_f64, -1.5_f64)]
#[case::the_default_applies_without_a_configured_value(None, 7.5_f64, 7.5_f64)]
fn timeout_resolution_prefers_the_configured_value(
    #[case] configured: Option<f64>,
    #[case] default_seconds: f64,
    #[case] expected: f64,
) {
    let resolved = resolve_embedding_timeout(configured, default_seconds);
    assert_eq!(resolved.to_bits(), expected.to_bits());
}

#[tokio::test]
async fn truncate_returns_the_prompt_without_a_limit_before_any_selection() {
    let prompt = "hello world";
    let truncated = truncate_embedding_input(&support::failing_selector(), prompt, MODEL, None)
        .await
        .unwrap();
    assert_eq!(truncated, prompt);
}

// the anthropic tokenizer decodes the <EOT> special token away, so a decode round trip changes the prompt
#[rstest]
#[case::at_the_limit(3, "<EOT>hello world")]
#[case::below_the_limit(5, "<EOT>hello world")]
#[tokio::test]
async fn truncate_returns_the_prompt_when_decoding_would_change_it(
    #[case] max_input_tokens: i64,
    #[case] expected: &str,
) {
    let prompt = "<EOT>hello world";
    let truncated = truncate_embedding_input(
        &support::selector(),
        prompt,
        "claude-2.1",
        Some(max_input_tokens),
    )
    .await
    .unwrap();
    assert_eq!(truncated, expected);
}

#[tokio::test]
async fn truncate_reports_a_failed_tokenizer_selection() {
    let error =
        truncate_embedding_input(&support::failing_selector(), "hello world", MODEL, Some(1))
            .await
            .unwrap_err();
    let EmbeddingInputError::Select { model, source } = error else {
        panic!("expected a tokenizer selection error");
    };
    assert_eq!(model, MODEL);
    assert!(matches!(
        *source,
        SelectionError::Engine(EngineError::PoolClosed)
    ));
}

#[rstest]
#[case::at_the_limit("hello world", 2, "hello world")]
#[case::below_the_limit("hello world", 5, "hello world")]
#[case::above_the_limit("hello world", 1, "hello")]
#[case::zero_decodes_nothing("hello world", 0, "")]
#[case::negative_drops_from_the_end("hello world", -1, "hello")]
#[case::negative_past_the_start("hello world", -5, "")]
#[tokio::test]
async fn truncate_keeps_the_first_tokens_of_a_python_slice(
    #[case] prompt: &str,
    #[case] max_input_tokens: i64,
    #[case] expected: &str,
) {
    let truncated =
        truncate_embedding_input(&support::selector(), prompt, MODEL, Some(max_input_tokens))
            .await
            .unwrap();
    assert_eq!(truncated, expected);
}
