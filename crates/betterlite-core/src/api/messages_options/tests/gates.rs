use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_anthropic::anthropic_capability_catalog::sampling::SamplingError;
use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;

use super::super::error::MessagesOptionsError;
use super::super::{ModelGate, requested_optional_params};
use super::support::{Catalogs, object};

const NO_DROP: DropParams = DropParams {
    global: false,
    call: false,
};
const CALL_DROP: DropParams = DropParams {
    global: false,
    call: true,
};
const GLOBAL_DROP: DropParams = DropParams {
    global: true,
    call: false,
};

fn run(
    model: &str,
    drop: DropParams,
    sent: &str,
) -> Result<IndexMap<String, Json>, MessagesOptionsError> {
    let catalogs = Catalogs::new();
    let sources = catalogs.sources();
    let gate = ModelGate {
        model,
        drop,
        custom_llm_provider: Some("anthropic"),
        catalogs: &sources,
    };
    requested_optional_params(&object(sent), Some(&gate))
}

fn rejected_param(error: &MessagesOptionsError) -> &str {
    match error {
        MessagesOptionsError::Sampling(SamplingError::Unsupported { param, .. }) => param,
        MessagesOptionsError::Speed(_) => "speed",
        MessagesOptionsError::Sampling(SamplingError::Value { .. })
        | MessagesOptionsError::MockInternalServer { .. }
        | MessagesOptionsError::MockContextWindowExceeded { .. }
        | MessagesOptionsError::MockRateLimit { .. } => "other",
    }
}

#[test]
fn unsupported_speed_raises_a_400_without_drop_params() {
    let error = run("claude-sonnet-4-5", NO_DROP, r#"{"speed": "fast"}"#).unwrap_err();
    assert!(matches!(error, MessagesOptionsError::Speed(_)));
    assert!(matches!(
        error.classify(),
        Some(ErrorClass::UnsupportedParams(_))
    ));
}

#[rstest]
#[case::call_flag(CALL_DROP)]
#[case::global_flag(GLOBAL_DROP)]
fn unsupported_speed_is_removed_when_dropping(#[case] drop: DropParams) {
    let kept = run(
        "claude-sonnet-4-5",
        drop,
        r#"{"speed": "fast", "max_tokens": 5}"#,
    )
    .expect("dropped");
    assert_eq!(kept, object(r#"{"max_tokens": 5}"#));
}

#[test]
fn supported_speed_survives() {
    let kept = run("claude-opus-4-8", NO_DROP, r#"{"speed": "fast"}"#).expect("kept");
    assert_eq!(kept, object(r#"{"speed": "fast"}"#));
}

#[rstest]
#[case::temperature_zero(r#"{"temperature": 0}"#, "temperature")]
#[case::top_p(r#"{"top_p": 0.5}"#, "top_p")]
#[case::top_k(r#"{"top_k": 2}"#, "top_k")]
fn sampling_params_fail_on_a_model_without_sampling(#[case] sent: &str, #[case] param: &str) {
    let error = run("claude-opus-4-7", NO_DROP, sent).unwrap_err();
    assert_eq!(rejected_param(&error), param);
    assert!(matches!(
        error.classify(),
        Some(ErrorClass::UnsupportedParams(_))
    ));
}

#[rstest]
#[case::call_flag(CALL_DROP)]
#[case::global_flag(GLOBAL_DROP)]
fn sampling_params_disappear_when_dropping(#[case] drop: DropParams) {
    let sent = r#"{"temperature": 0, "top_p": 0.5, "top_k": 2, "max_tokens": 9}"#;
    let kept = run("claude-opus-4-7", drop, sent).expect("dropped");
    assert_eq!(kept, object(r#"{"max_tokens": 9}"#));
}

#[test]
fn temperature_one_survives_a_model_without_sampling() {
    let kept = run("claude-opus-4-7", NO_DROP, r#"{"temperature": 1}"#).expect("kept");
    assert_eq!(kept, object(r#"{"temperature": 1}"#));
}

#[test]
fn a_model_with_sampling_keeps_every_sampling_param() {
    let sent = r#"{"temperature": 0, "top_p": 0.5, "top_k": 2}"#;
    assert_eq!(
        run("claude-sonnet-4-5", NO_DROP, sent).expect("kept"),
        object(sent)
    );
}

#[test]
fn sampling_params_move_to_the_end_in_temperature_top_p_top_k_order() {
    let sent = r#"{"top_k": 2, "max_tokens": 9, "top_p": 0.5, "temperature": 0, "stream": true}"#;
    let kept = run("claude-sonnet-4-5", NO_DROP, sent).expect("kept");
    assert_eq!(
        kept.keys().map(String::as_str).collect::<Vec<_>>(),
        ["max_tokens", "stream", "temperature", "top_p", "top_k"]
    );
}

#[rstest]
#[case::speed_first(
    r#"{"top_k": 2, "top_p": 0.5, "temperature": 0, "speed": "fast"}"#,
    "speed"
)]
#[case::temperature_second(r#"{"top_k": 2, "top_p": 0.5, "temperature": 0}"#, "temperature")]
#[case::top_p_third(r#"{"top_k": 2, "top_p": 0.5, "temperature": 1}"#, "top_p")]
#[case::top_k_last(r#"{"top_k": 2, "temperature": 1}"#, "top_k")]
fn the_first_failing_gate_names_the_error(#[case] sent: &str, #[case] rejected: &str) {
    let error = run("claude-opus-4-7", NO_DROP, sent).unwrap_err();
    assert_eq!(rejected_param(&error), rejected);
}

#[test]
fn null_gated_params_are_dropped_before_the_gates() {
    let kept = run(
        "claude-opus-4-7",
        NO_DROP,
        r#"{"speed": null, "top_p": null}"#,
    )
    .expect("nulls never reach the gates");
    assert!(kept.is_empty());
}

#[test]
fn without_a_model_every_gated_param_is_kept() {
    let sent = r#"{"speed": "fast", "temperature": 0, "top_p": 0.5, "top_k": 2}"#;
    assert_eq!(
        requested_optional_params(&object(sent), None).expect("kept"),
        object(sent)
    );
}
