use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use crate::regional_host_dispatch::error::DispatchError;

use super::embedding::call_for;
use super::support::{EmbeddingFixture, secret, secret_text};

const QWENCLOUD_MESSAGE: &str = "Missing API key for QwenCloud. Set QWENCLOUD_API_KEY or DASHSCOPE_API_KEY environment variable or pass api_key parameter.";

// QwenCloud reads its own key before DASHSCOPE_API_KEY, and the request key before either
#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-brand"),
    Some("sk-dashscope"),
    Some("sk-request")
)]
#[case::brand_key(None, Some("sk-brand"), Some("sk-dashscope"), Some("sk-brand"))]
#[case::dashscope_key(None, None, Some("sk-dashscope"), Some("sk-dashscope"))]
fn qwencloud_key_chain(
    #[case] request_key: Option<&str>,
    #[case] brand_key: Option<&str>,
    #[case] dashscope_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = EmbeddingFixture::new();
    fixture.api_key = request_key.map(secret);
    fixture.settings.environment.qwencloud_key = brand_key.map(secret);
    fixture.settings.environment.dashscope_api_key = dashscope_key.map(secret);
    let call = call_for(ProviderId::Qwencloud, &fixture).expect("the arm answers");
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        expected.map(str::to_owned)
    );
}

#[test]
fn qwencloud_without_a_key_names_its_own_variables() {
    let fixture = EmbeddingFixture::new();
    let outcome = call_for(ProviderId::Qwencloud, &fixture);
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey {
            message: QWENCLOUD_MESSAGE
        })
    ));
}
