use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use crate::regional_host_dispatch::error::DispatchError;

use super::embedding::call_for;
use super::support::{EmbeddingFixture, secret};

const QWEN_AI_PLATFORM_MESSAGE: &str = "Missing API key for Qianwen AI Platform. Set QWEN_AI_PLATFORM_API_KEY or DASHSCOPE_API_KEY environment variable or pass api_key parameter.";

// Qianwen AI Platform reads its own key before DASHSCOPE_API_KEY, the request key before either, and litellm.api_key last
#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-brand"),
    Some("sk-dashscope"),
    Some("sk-global"),
    Some("sk-request")
)]
#[case::brand_key(None, Some("sk-brand"), None, None, Some("sk-brand"))]
#[case::brand_key_beats_the_dashscope_key_and_the_global_key(
    None,
    Some("sk-brand"),
    Some("sk-dashscope"),
    Some("sk-global"),
    Some("sk-brand")
)]
#[case::dashscope_key(None, None, Some("sk-dashscope"), None, Some("sk-dashscope"))]
#[case::dashscope_key_beats_the_global_key(
    None,
    None,
    Some("sk-dashscope"),
    Some("sk-global"),
    Some("sk-dashscope")
)]
#[case::global_key(None, None, None, Some("sk-global"), Some("sk-global"))]
fn qwen_ai_platform_key_chain(
    #[case] request_key: Option<&str>,
    #[case] brand_key: Option<&str>,
    #[case] dashscope_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = EmbeddingFixture::baseline();
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.environment.qwen_ai_platform_key = brand_key.map(secret);
    fixture.settings.environment.dashscope_api_key = dashscope_key.map(secret);
    let call = call_for(ProviderId::QwenAiPlatform, &fixture).expect("the arm answers");
    assert_eq!(
        call.api_key
            .as_ref()
            .map(|key| key.expose_secret().to_owned()),
        expected.map(str::to_owned)
    );
}

#[test]
fn qwen_ai_platform_without_a_key_names_its_own_variables() {
    let fixture = EmbeddingFixture::baseline();
    let outcome = call_for(ProviderId::QwenAiPlatform, &fixture);
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey {
            message: QWEN_AI_PLATFORM_MESSAGE
        })
    ));
}
