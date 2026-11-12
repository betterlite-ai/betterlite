use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::fixture::{Fixture, capture_warnings};
use super::support::json;
use crate::anthropic_capability_catalog::DropParams;
use crate::capabilities::speed::{
    DROP_UNSUPPORTED_SPEED_WARNING, SpeedDecision, SpeedParam, SpeedRequest,
    maybe_drop_speed_param, model_supports_speed_param,
};

const SPEED_MODELS: &str = r#"{
    "claude-opus-4-8": {"litellm_provider": "anthropic", "supports_speed": true},
    "claude-opus-4-7": {"litellm_provider": "anthropic", "supports_speed": false},
    "claude-opus-4-6": {"litellm_provider": "anthropic", "supports_fast_mode": true},
    "anthropic/claude-opus-4-5": {"litellm_provider": "anthropic", "supports_speed": true}
}"#;

const SPEED_PACKAGED: &str = r#"{
    "claude-packaged": {"supports_speed": true}
}"#;

const NO_DROP: DropParams = DropParams {
    global: false,
    call: false,
};
const GLOBAL_DROP: DropParams = DropParams {
    global: true,
    call: false,
};
const CALL_DROP: DropParams = DropParams {
    global: false,
    call: true,
};

fn supports_speed(fixture: &Fixture, model: &str, provider: Option<&str>) -> bool {
    model_supports_speed_param(model, provider, &fixture.catalogs()).expect("catalog lookup")
}

fn drop_request<'request>(
    model: &'request str,
    value: &'request Json,
    drop: DropParams,
) -> SpeedRequest<'request> {
    SpeedRequest {
        model,
        speed: SpeedParam::Present(value),
        drop,
        custom_llm_provider: Some("anthropic"),
    }
}

fn speed_support_needs_the_exact_key_and_host(fixture: &Fixture) {
    assert!(supports_speed(fixture, "claude-opus-4-8", None));
    assert!(supports_speed(
        fixture,
        "claude-opus-4-8",
        Some("anthropic")
    ));
    assert!(!supports_speed(fixture, "claude-opus-4-8", Some("bedrock")));
    assert!(!supports_speed(
        fixture,
        "claude-opus-4-8",
        Some("vertex_ai")
    ));
    assert!(!supports_speed(
        fixture,
        "claude-opus-4-7",
        Some("anthropic")
    ));
}

fn speed_support_never_infers_from_aliases(fixture: &Fixture) {
    assert!(
        !supports_speed(fixture, "claude-opus-4-6", Some("anthropic")),
        "supports_fast_mode never stands in for the exact supports_speed key"
    );
    assert!(
        !supports_speed(fixture, "claude-opus-4-5", Some("anthropic")),
        "the prefixed entry is not an alias for the bare model"
    );
    assert!(
        !supports_speed(fixture, "claude-packaged", Some("anthropic")),
        "the exact lookup never reads the packaged catalog"
    );
}

fn speed_absent_is_a_no_op(fixture: &Fixture) {
    let absent = SpeedRequest {
        model: "claude-opus-4-7",
        speed: SpeedParam::Absent,
        drop: NO_DROP,
        custom_llm_provider: Some("anthropic"),
    };
    assert_eq!(
        maybe_drop_speed_param(&absent, &fixture.catalogs()).expect("absent speed"),
        SpeedDecision::Keep
    );
    let fast = json(r#""fast""#);
    assert_eq!(
        maybe_drop_speed_param(
            &drop_request("claude-opus-4-8", &fast, NO_DROP),
            &fixture.catalogs()
        )
        .expect("supported speed"),
        SpeedDecision::Keep
    );
}

fn speed_unsupported_raises(fixture: &Fixture) {
    let fast = json(r#""fast""#);
    let refused = maybe_drop_speed_param(
        &drop_request("claude-opus-4-7", &fast, NO_DROP),
        &fixture.catalogs(),
    )
    .expect_err("no drop_params");
    assert_eq!(refused.status_code(), Some(400));
    assert!(matches!(
        refused.classify(),
        Some(ErrorClass::UnsupportedParams(_))
    ));
    assert!(
        refused
            .to_string()
            .contains("does not support speed=\"fast\""),
        "the message names the value: {refused}"
    );
    let null_speed = json("null");
    let null_refused = maybe_drop_speed_param(
        &drop_request("claude-opus-4-7", &null_speed, NO_DROP),
        &fixture.catalogs(),
    )
    .expect_err("a present null speed still raises");
    assert_eq!(null_refused.status_code(), Some(400));
}

fn speed_drop_flags_warn_and_remove(fixture: &Fixture) {
    let fast = json(r#""fast""#);
    let expected = [DROP_UNSUPPORTED_SPEED_WARNING.replacen("%s", "claude-opus-4-7", 1)];
    let (guard, warnings) = capture_warnings();
    assert_eq!(
        maybe_drop_speed_param(
            &drop_request("claude-opus-4-7", &fast, GLOBAL_DROP),
            &fixture.catalogs()
        )
        .expect("global drop"),
        SpeedDecision::Drop
    );
    assert_eq!(warnings.messages(), expected);
    drop(guard);
    let (call_guard, call_warnings) = capture_warnings();
    assert_eq!(
        maybe_drop_speed_param(
            &drop_request("claude-opus-4-7", &fast, CALL_DROP),
            &fixture.catalogs()
        )
        .expect("per-call drop"),
        SpeedDecision::Drop
    );
    assert_eq!(call_warnings.messages(), expected);
    drop(call_guard);
}

fn speed_routed_to_another_provider_raises(fixture: &Fixture) {
    let fast = json(r#""fast""#);
    let routed = SpeedRequest {
        model: "claude-opus-4-8",
        speed: SpeedParam::Present(&fast),
        drop: NO_DROP,
        custom_llm_provider: Some("bedrock"),
    };
    let refused = maybe_drop_speed_param(&routed, &fixture.catalogs())
        .expect_err("a routed non-Anthropic provider cannot use fast mode");
    assert_eq!(refused.status_code(), Some(400));
}

pub(super) fn speed_policy() {
    let fixture = Fixture::new(SPEED_MODELS, SPEED_PACKAGED);
    speed_support_needs_the_exact_key_and_host(&fixture);
    speed_support_never_infers_from_aliases(&fixture);
    speed_absent_is_a_no_op(&fixture);
    speed_unsupported_raises(&fixture);
    speed_drop_flags_warn_and_remove(&fixture);
    speed_routed_to_another_provider_raises(&fixture);
}
