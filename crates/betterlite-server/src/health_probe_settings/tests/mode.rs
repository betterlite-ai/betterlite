use rstest::rstest;

use crate::health_probe_settings::mode::{
    MAX_TOKEN_SUPPORT_MODES, REASONING_EFFORT_MODES, resolve_health_check_mode,
    should_inject_max_tokens,
};
use crate::health_probe_settings::tests::support::{StubModels, health, object};

#[test]
fn the_mode_constants_match_litellm() {
    assert_eq!(MAX_TOKEN_SUPPORT_MODES, ["chat", "completion", "responses"]);
    assert_eq!(
        REASONING_EFFORT_MODES,
        [None, Some("chat"), Some("completion")]
    );
}

#[test]
fn an_explicit_model_info_mode_wins_over_the_cost_map() {
    let models = StubModels::with_mode("gpt-4o", "chat");
    let model_info = object(r#"{"mode":"embedding"}"#);
    let params = object(r#"{"model":"gpt-4o"}"#);
    assert_eq!(
        resolve_health_check_mode(&model_info, &params, &models),
        Some("embedding".to_owned())
    );
}

#[test]
fn the_cost_map_answers_when_model_info_has_no_mode() {
    let models = StubModels::with_mode("text-embedding-3", "embedding");
    let params = object(r#"{"model":"text-embedding-3"}"#);
    assert_eq!(
        resolve_health_check_mode(&object("{}"), &params, &models),
        Some("embedding".to_owned())
    );
}

#[rstest]
#[case::a_number(r#"{"mode":3}"#)]
#[case::null(r#"{"mode":null}"#)]
fn a_non_text_mode_counts_as_unset(#[case] model_info: &str) {
    let models = StubModels::with_mode("text-embedding-3", "embedding");
    let params = object(r#"{"model":"text-embedding-3"}"#);
    assert_eq!(
        resolve_health_check_mode(&object(model_info), &params, &models),
        Some("embedding".to_owned())
    );
}

#[rstest]
#[case::unknown_model(r#"{"model":"mystery-model"}"#)]
#[case::missing_model("{}")]
#[case::non_text_model(r#"{"model":7}"#)]
fn an_unresolvable_mode_is_none(#[case] params: &str) {
    let models = StubModels::default();
    assert_eq!(
        resolve_health_check_mode(&object("{}"), &object(params), &models),
        None
    );
}

#[rstest]
#[case::explicit_false(Some(false), Some("chat"), false)]
#[case::explicit_true(Some(true), Some("embedding"), true)]
fn the_operator_override_wins_over_the_mode(
    #[case] supports_max_tokens: Option<bool>,
    #[case] mode: Option<&str>,
    #[case] expected: bool,
) {
    let health = health(None, None, None, supports_max_tokens);
    assert_eq!(should_inject_max_tokens(&health, mode), expected);
}

#[rstest]
#[case::chat("chat")]
#[case::completion("completion")]
#[case::responses("responses")]
fn chat_style_modes_take_max_tokens(#[case] mode: &str) {
    assert!(should_inject_max_tokens(
        &health(None, None, None, None),
        Some(mode)
    ));
}

#[rstest]
#[case::embedding("embedding")]
#[case::audio_speech("audio_speech")]
#[case::image_generation("image_generation")]
fn other_modes_do_not_take_max_tokens(#[case] mode: &str) {
    assert!(!should_inject_max_tokens(
        &health(None, None, None, None),
        Some(mode)
    ));
}

#[test]
fn an_unresolvable_mode_takes_max_tokens_as_chat() {
    assert!(should_inject_max_tokens(
        &health(None, None, None, None),
        None
    ));
}
