use betterlite_error::error::ErrorClass;

use super::fixture::{Fixture, Session};
use crate::capabilities::effort::{EffortError, validate_effort_for_model};
use crate::capabilities::effort_param::model_supports_effort_param;

const EFFORT_MODELS: &str = r#"{
    "anthropic/claude-opus-4-7": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "anthropic/claude-opus-4-8": {"litellm_provider": "anthropic", "supports_max_reasoning_effort": true, "supports_xhigh_reasoning_effort": true},
    "claude-config-model": {"litellm_provider": "anthropic", "supports_output_config": true},
    "claude-low": {"litellm_provider": "anthropic", "supports_low_reasoning_effort": true},
    "claude-minimal": {"litellm_provider": "anthropic", "supports_minimal_reasoning_effort": true},
    "claude-medium": {"litellm_provider": "anthropic", "supports_medium_reasoning_effort": true},
    "claude-high": {"litellm_provider": "anthropic", "supports_high_reasoning_effort": true},
    "claude-xhigh": {"litellm_provider": "anthropic", "supports_xhigh_reasoning_effort": true},
    "claude-max": {"litellm_provider": "anthropic", "supports_max_reasoning_effort": true},
    "claude-haiku-4-5": {"litellm_provider": "anthropic"}
}"#;

async fn validated(fixture: &Fixture, model: &str, effort: Option<&str>) -> Option<String> {
    let mut session = Session::new(fixture);
    validate_effort_for_model(
        model,
        effort,
        "anthropic",
        &mut session.environment(),
        &fixture.catalogs(),
    )
    .await
}

async fn accepts_effort(fixture: &Fixture, model: &str) -> bool {
    let mut session = Session::new(fixture);
    model_supports_effort_param(
        model,
        "anthropic",
        &mut session.environment(),
        &fixture.catalogs(),
    )
    .await
}

async fn max_is_gated_by_adaptive_or_the_level_flag(fixture: &Fixture) {
    assert_eq!(
        validated(fixture, "claude-opus-4-7", Some("max")).await,
        None,
        "adaptive thinking allows max"
    );
    assert_eq!(
        validated(fixture, "claude-opus-4-8", Some("max")).await,
        None,
        "the explicit max level flag allows max"
    );
    assert_eq!(
        validated(fixture, "claude-haiku-4-5", Some("max")).await,
        Some("effort='max' is not supported by this model. Got model: claude-haiku-4-5".to_owned())
    );
}

async fn xhigh_needs_its_own_level_flag(fixture: &Fixture) {
    assert_eq!(
        validated(fixture, "claude-opus-4-8", Some("xhigh")).await,
        None
    );
    assert_eq!(
        validated(fixture, "claude-opus-4-7", Some("xhigh")).await,
        Some(
            "effort='xhigh' is not supported by this model. Got model: claude-opus-4-7".to_owned()
        ),
        "adaptive thinking does not imply the xhigh level flag"
    );
}

async fn other_levels_and_absence_are_allowed(fixture: &Fixture) {
    assert_eq!(
        validated(fixture, "claude-haiku-4-5", Some("low")).await,
        None
    );
    assert_eq!(
        validated(fixture, "claude-haiku-4-5", Some("medium")).await,
        None
    );
    assert_eq!(
        validated(fixture, "claude-haiku-4-5", Some("high")).await,
        None
    );
    assert_eq!(
        validated(fixture, "claude-haiku-4-5", Some("minimal")).await,
        None
    );
    assert_eq!(validated(fixture, "claude-haiku-4-5", None).await, None);
}

async fn every_level_flag_accepts_the_effort_param(fixture: &Fixture) {
    let models = [
        "claude-low",
        "claude-minimal",
        "claude-medium",
        "claude-high",
        "claude-xhigh",
        "claude-max",
    ];
    for model in models {
        assert!(
            accepts_effort(fixture, model).await,
            "the {model} level flag"
        );
    }
}

async fn effort_param_support_accepts_output_config_or_a_level(fixture: &Fixture) {
    assert!(accepts_effort(fixture, "claude-opus-4-8").await);
    assert!(accepts_effort(fixture, "claude-config-model").await);
    every_level_flag_accepts_the_effort_param(fixture).await;
    assert!(!accepts_effort(fixture, "claude-haiku-4-5").await);
}

fn invalid_effort_is_a_bad_request_with_model_and_provider() {
    let invalid = EffortError::InvalidReasoningEffort {
        value: "bogus".to_owned(),
        model: "claude-opus-4-8".to_owned(),
        provider: "anthropic".to_owned(),
    };
    assert_eq!(
        invalid.to_string(),
        "Invalid reasoning_effort: bogus. Must be one of: 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'none'."
    );
    let class = invalid.classify();
    assert!(matches!(class, ErrorClass::BadRequest(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.model.as_deref(), Some("claude-opus-4-8"));
    assert_eq!(metadata.provider.as_deref(), Some("anthropic"));
}

pub(super) async fn effort_validation() {
    let fixture = Fixture::new(EFFORT_MODELS, "{}");
    max_is_gated_by_adaptive_or_the_level_flag(&fixture).await;
    xhigh_needs_its_own_level_flag(&fixture).await;
    other_levels_and_absence_are_allowed(&fixture).await;
    effort_param_support_accepts_output_config_or_a_level(&fixture).await;
    invalid_effort_is_a_bad_request_with_model_and_provider();
}
