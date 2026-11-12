use super::fixture::{Fixture, Session};
use crate::capabilities::supported_params::{BASE_SUPPORTED_PARAMS, get_supported_openai_params};

const SUPPORTED_MODELS: &str = r#"{
    "anthropic/claude-opus-4-7": {"litellm_provider": "anthropic", "supports_adaptive_thinking": true},
    "claude-sonnet-4-5": {"litellm_provider": "anthropic", "supports_reasoning": true},
    "claude-haiku-4-5": {"litellm_provider": "anthropic"},
    "claude-3-7-sonnet-20250219": {"litellm_provider": "anthropic"}
}"#;

async fn supported(fixture: &Fixture, model: &str) -> Vec<&'static str> {
    let mut session = Session::new(fixture);
    get_supported_openai_params(
        model,
        Some("anthropic"),
        &mut session.environment(),
        &fixture.catalogs(),
    )
    .await
}

fn with_thinking() -> Vec<&'static str> {
    let mut expected = BASE_SUPPORTED_PARAMS.to_vec();
    expected.push("thinking");
    expected.push("reasoning_effort");
    expected
}

async fn thinking_is_advertised(fixture: &Fixture) {
    assert_eq!(
        supported(fixture, "claude-3-7-sonnet-20250219").await,
        with_thinking(),
        "the case-sensitive substring appends thinking"
    );
    assert_eq!(
        supported(fixture, "claude-opus-4-7").await,
        with_thinking(),
        "adaptive thinking appends thinking"
    );
    assert_eq!(
        supported(fixture, "claude-sonnet-4-5").await,
        with_thinking(),
        "supports_reasoning appends thinking"
    );
}

async fn the_base_list_is_unchanged_for_other_models(fixture: &Fixture) {
    assert_eq!(
        supported(fixture, "Claude-3-7-Sonnet-20250219").await,
        BASE_SUPPORTED_PARAMS.to_vec(),
        "the substring check is case-sensitive"
    );
    assert_eq!(
        supported(fixture, "claude-haiku-4-5").await,
        BASE_SUPPORTED_PARAMS.to_vec(),
        "speed stays advertised even though the speed gate rejects this model"
    );
}

pub(super) async fn supported_parameter_list() {
    let fixture = Fixture::new(SUPPORTED_MODELS, "{}");
    thinking_is_advertised(&fixture).await;
    the_base_list_is_unchanged_for_other_models(&fixture).await;
}
