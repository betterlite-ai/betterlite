use betterlite_models::identity::provider_id::ProviderId;
use rstest::rstest;

use crate::regional_host_dispatch::error::DispatchError;

use super::embedding::call_for;
use super::support::{EmbeddingFixture, secret, secret_text};

const DASHSCOPE_MESSAGE: &str = "Missing API key for DashScope. Set DASHSCOPE_API_KEY environment variable or pass api_key parameter.";

#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-global"),
    Some("sk-dashscope"),
    Some("sk-request")
)]
#[case::global_key(None, Some("sk-global"), Some("sk-dashscope"), Some("sk-global"))]
#[case::dashscope_env(None, None, Some("sk-dashscope"), Some("sk-dashscope"))]
fn dashscope_key_chain(
    #[case] request_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] dashscope_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = EmbeddingFixture::new();
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.environment.dashscope_api_key = dashscope_key.map(secret);
    let call = call_for(ProviderId::Dashscope, &fixture).expect("the arm answers");
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        expected.map(str::to_owned)
    );
}

#[test]
fn dashscope_without_a_key_names_only_dashscope() {
    let fixture = EmbeddingFixture::new();
    let outcome = call_for(ProviderId::Dashscope, &fixture);
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey {
            message: DASHSCOPE_MESSAGE
        })
    ));
}
