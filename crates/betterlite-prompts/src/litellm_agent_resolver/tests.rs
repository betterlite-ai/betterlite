#![cfg(test)]

use betterlite_models::decisions::Json;
use betterlite_models::prompt_contracts::{AllMessageValues, PromptParams};
use indexmap::IndexMap;
use rstest::rstest;
use serde_json::Number;

use super::{LitellmAgentResolver, is_litellm_agent_model};

fn sample_messages() -> Vec<AllMessageValues> {
    vec![Json::Object(IndexMap::from([(
        "role".to_owned(),
        Json::Text("user".to_owned()),
    )]))]
}

fn sample_params() -> PromptParams {
    IndexMap::from([
        ("temperature".to_owned(), Json::Number(Number::from(1_i64))),
        ("stream".to_owned(), Json::Bool(false)),
    ])
}

#[rstest]
#[case::leading("litellm_agent/gpt-4", "gpt-4")]
#[case::after_a_provider("openai/litellm_agent/gpt-4", "openai/gpt-4")]
#[case::only_the_first("litellm_agent/litellm_agent/x", "litellm_agent/x")]
#[case::prefix_only("litellm_agent/", "")]
#[case::absent("gpt-4", "gpt-4")]
#[case::without_the_slash("litellm_agent", "litellm_agent")]
#[case::slash_without_the_prefix("gpt-4/litellm_agent", "gpt-4/litellm_agent")]
#[case::uppercase("LITELLM_AGENT/gpt-4", "LITELLM_AGENT/gpt-4")]
#[case::space_before_the_prefix(" litellm_agent/gpt-4", " gpt-4")]
#[case::space_after_the_prefix("litellm_agent/ gpt-4", " gpt-4")]
#[case::trailing_space("litellm_agent/gpt-4 ", "gpt-4 ")]
#[case::empty("", "")]
fn the_first_prefix_anywhere_is_stripped_once(#[case] model: &str, #[case] expected: &str) {
    let resolver = LitellmAgentResolver;
    let (resolved, messages, params) =
        resolver.get_chat_completion_prompt(model, sample_messages(), sample_params(), false);
    assert_eq!(resolved, expected);
    assert_eq!(messages, sample_messages());
    assert_eq!(params, sample_params());
}

#[rstest]
#[case::leading("litellm_agent/gpt-4")]
#[case::after_a_provider("openai/litellm_agent/gpt-4")]
#[case::only_the_first("litellm_agent/litellm_agent/x")]
fn the_ignore_flag_keeps_the_model(#[case] model: &str) {
    let resolver = LitellmAgentResolver;
    let (resolved, messages, params) =
        resolver.get_chat_completion_prompt(model, sample_messages(), sample_params(), true);
    assert_eq!(resolved, model);
    assert_eq!(messages, sample_messages());
    assert_eq!(params, sample_params());
}

#[rstest]
#[case::stripped("litellm_agent/gpt-4", false)]
#[case::ignored("litellm_agent/gpt-4", true)]
#[case::unchanged("gpt-4", false)]
#[tokio::test]
async fn the_async_method_returns_the_sync_result(#[case] model: &str, #[case] ignore: bool) {
    let resolver = LitellmAgentResolver;
    let expected =
        resolver.get_chat_completion_prompt(model, sample_messages(), sample_params(), ignore);
    let actual = resolver
        .async_completion_prompt(model, sample_messages(), sample_params(), ignore)
        .await;
    assert_eq!(actual, expected);
}

#[rstest]
#[case::leading("litellm_agent/gpt-4", true)]
#[case::prefix_only("litellm_agent/", true)]
#[case::after_a_provider("openai/litellm_agent/gpt-4", false)]
#[case::plain("gpt-4", false)]
#[case::without_the_slash("litellm_agent", false)]
#[case::uppercase("LITELLM_AGENT/gpt-4", false)]
#[case::leading_space(" litellm_agent/gpt-4", false)]
#[case::empty("", false)]
fn the_agent_gate_matches_a_leading_prefix_only(#[case] model: &str, #[case] expected: bool) {
    assert_eq!(is_litellm_agent_model(model), expected);
}
