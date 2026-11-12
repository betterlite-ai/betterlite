use betterlite_error::error::ErrorClass;
use betterlite_models::decisions::Json;

use super::fixture::{Fixture, Session};
use super::support::json;
use crate::capabilities::ThinkingBudgets;
use crate::capabilities::reasoning::{
    ReasoningEffortError, ReasoningRequest, map_reasoning_effort,
};

const THINKING_MODELS: &str = r#"{
    "anthropic/claude-opus-4-7": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic"}
}"#;

async fn mapped(
    fixture: &Fixture,
    model: &str,
    effort: Option<&str>,
) -> Result<Option<Json>, ReasoningEffortError> {
    let budgets = ThinkingBudgets::default();
    let mut session = Session::new(fixture);
    let request = ReasoningRequest {
        reasoning_effort: effort,
        model,
        custom_llm_provider: "anthropic",
        llm_provider: "anthropic",
        budgets: &budgets,
    };
    map_reasoning_effort(&request, &mut session.environment(), &fixture.catalogs()).await
}

fn enabled(budget: i64) -> Json {
    json(&format!(
        r#"{{"type": "enabled", "budget_tokens": {budget}}}"#
    ))
}

async fn minimal_none_maps_to_no_thinking(fixture: &Fixture) {
    assert_eq!(mapped(fixture, "claude-opus-4-7", None).await, Ok(None));
    assert_eq!(
        mapped(fixture, "claude-opus-4-7", Some("none")).await,
        Ok(None)
    );
}

async fn adaptive_model_answers_before_validation(fixture: &Fixture) {
    let adaptive = Some(json(r#"{"type": "adaptive", "display": "summarized"}"#));
    assert_eq!(
        mapped(fixture, "claude-opus-4-7", Some("low")).await,
        Ok(adaptive.clone())
    );
    assert_eq!(
        mapped(fixture, "claude-opus-4-7", Some("max")).await,
        Ok(adaptive.clone())
    );
    assert_eq!(
        mapped(fixture, "claude-opus-4-7", Some("bogus")).await,
        Ok(adaptive),
        "the adaptive branch returns before the effort string is validated"
    );
}

async fn legacy_levels_use_the_configured_budgets(fixture: &Fixture) {
    let expectations = [
        ("minimal", 1024_i64),
        ("low", 1024),
        ("medium", 2048),
        ("high", 4096),
        ("xhigh", 8192),
        ("max", 0x4000),
    ];
    for (effort, budget) in expectations {
        assert_eq!(
            mapped(fixture, "claude-sonnet-4-5", Some(effort)).await,
            Ok(Some(enabled(budget))),
            "the {effort} budget"
        );
    }
}

async fn unknown_effort_is_a_bad_request(fixture: &Fixture) {
    let unknown = mapped(fixture, "claude-sonnet-4-5", Some("bogus"))
        .await
        .expect_err("an unknown effort is rejected");
    assert_eq!(
        unknown.to_string(),
        "Unmapped reasoning effort: bogus. Must be one of: 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'none'."
    );
    let class = unknown.classify();
    assert!(matches!(class, ErrorClass::BadRequest(_)));
    let metadata = class.metadata();
    assert_eq!(metadata.status_code, 400);
    assert_eq!(metadata.model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(metadata.provider.as_deref(), Some("anthropic"));
}

async fn a_configured_minimal_budget_is_used_as_given(fixture: &Fixture) {
    let raised = ThinkingBudgets {
        minimal: 4096,
        low: 1024,
        medium: 2048,
        high: 4096,
        xhigh: 8192,
        max: 0x4000,
    };
    let mut session = Session::new(fixture);
    let request = ReasoningRequest {
        reasoning_effort: Some("minimal"),
        model: "claude-sonnet-4-5",
        custom_llm_provider: "anthropic",
        llm_provider: "anthropic",
        budgets: &raised,
    };
    assert_eq!(
        map_reasoning_effort(&request, &mut session.environment(), &fixture.catalogs()).await,
        Ok(Some(enabled(4096)))
    );
}

pub(super) async fn minimal_thinking_floor() {
    let fixture = Fixture::new(THINKING_MODELS, "{}");
    minimal_none_maps_to_no_thinking(&fixture).await;
    adaptive_model_answers_before_validation(&fixture).await;
    legacy_levels_use_the_configured_budgets(&fixture).await;
    unknown_effort_is_a_bad_request(&fixture).await;
    a_configured_minimal_budget_is_used_as_given(&fixture).await;
}
