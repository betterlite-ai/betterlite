use rstest::rstest;

use crate::health_probe_settings::max_tokens::{probe_model_string, resolve_max_tokens};
use crate::health_probe_settings::tests::support::{StubModels, health, object, policy};

fn reasoning_models() -> StubModels {
    let mut models = StubModels::default();
    models.reasoning.insert("reasoning-model".to_owned());
    models
}

#[test]
fn the_explicit_override_wins_over_every_other_source() {
    let tokens = resolve_max_tokens(
        &health(Some(42), Some(7), Some(8), None),
        &object(r#"{"model":"provider/*"}"#),
        &policy(Some(99), Some(50)),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(42));
}

#[test]
fn a_reasoning_route_takes_its_reasoning_limit() {
    let tokens = resolve_max_tokens(
        &health(None, Some(64), Some(8), None),
        &object(r#"{"model":"reasoning-model"}"#),
        &policy(None, None),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(64));
}

#[test]
fn a_non_reasoning_route_takes_its_non_reasoning_limit() {
    let tokens = resolve_max_tokens(
        &health(None, Some(64), Some(8), None),
        &object(r#"{"model":"plain-model"}"#),
        &policy(None, None),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(8));
}

#[test]
fn a_reasoning_route_without_its_own_limit_takes_the_environment_one() {
    let tokens = resolve_max_tokens(
        &health(None, None, Some(8), None),
        &object(r#"{"model":"reasoning-model"}"#),
        &policy(None, Some(70)),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(70));
}

#[test]
fn the_reasoning_environment_skips_non_reasoning_routes() {
    let tokens = resolve_max_tokens(
        &health(None, None, None, None),
        &object(r#"{"model":"plain-model"}"#),
        &policy(None, Some(70)),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(16));
}

#[test]
fn the_global_environment_applies_to_every_route() {
    let tokens = resolve_max_tokens(
        &health(None, None, None, None),
        &object(r#"{"model":"plain-model"}"#),
        &policy(Some(32), None),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(32));
}

#[test]
fn the_global_environment_reaches_wildcards() {
    let tokens = resolve_max_tokens(
        &health(None, None, None, None),
        &object(r#"{"model":"provider/*"}"#),
        &policy(Some(32), None),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(32));
}

#[test]
fn a_non_wildcard_route_defaults_to_sixteen() {
    let tokens = resolve_max_tokens(
        &health(None, None, None, None),
        &object(r#"{"model":"plain-model"}"#),
        &policy(None, None),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(16));
}

#[test]
fn a_wildcard_route_leaves_max_tokens_unset() {
    let tokens = resolve_max_tokens(
        &health(None, None, None, None),
        &object(r#"{"model":"provider/*"}"#),
        &policy(None, None),
        &reasoning_models(),
    );
    assert_eq!(tokens, None);
}

#[test]
fn a_wildcard_route_ignores_the_model_info_limits() {
    let tokens = resolve_max_tokens(
        &health(None, Some(64), Some(8), None),
        &object(r#"{"model":"provider/*"}"#),
        &policy(None, None),
        &reasoning_models(),
    );
    assert_eq!(tokens, None);
}

#[rstest]
#[case::only_the_reasoning_limit(r#"{"model":"reasoning-model"}"#, Some(64), None, 64)]
#[case::only_the_non_reasoning_limit(r#"{"model":"plain-model"}"#, None, Some(8), 8)]
fn a_single_variant_limit_still_applies(
    #[case] params: &str,
    #[case] reasoning: Option<i64>,
    #[case] non_reasoning: Option<i64>,
    #[case] expected: i64,
) {
    let tokens = resolve_max_tokens(
        &health(None, reasoning, non_reasoning, None),
        &object(params),
        &policy(None, None),
        &reasoning_models(),
    );
    assert_eq!(tokens, Some(expected));
}

#[test]
fn the_deployment_model_string_reads_a_text_model() {
    assert_eq!(
        probe_model_string(&object(r#"{"model":"openai/gpt"}"#)),
        "openai/gpt"
    );
}

#[rstest]
#[case::missing("{}")]
#[case::null(r#"{"model":null}"#)]
#[case::not_text(r#"{"model":5}"#)]
fn a_missing_model_reads_as_empty(#[case] params: &str) {
    assert_eq!(probe_model_string(&object(params)), "");
}
