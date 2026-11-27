use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Json;
use rstest::rstest;

use crate::hosted_platform_dispatch::call::EmbeddingCall;
use crate::regional_host_dispatch::embedding::dispatch;
use crate::regional_host_dispatch::error::DispatchError;

use super::support::{EmbeddingFixture, headers, secret, secret_text};

const VOLCENGINE_MESSAGE: &str = "Missing API key for Volcengine. Set ARK_API_KEY or VOLCENGINE_API_KEY environment variable or pass api_key parameter.";
const DASHSCOPE_MESSAGE: &str = "Missing API key for DashScope. Set DASHSCOPE_API_KEY environment variable or pass api_key parameter.";
const QWENCLOUD_MESSAGE: &str = "Missing API key for QwenCloud. Set QWENCLOUD_API_KEY or DASHSCOPE_API_KEY environment variable or pass api_key parameter.";
const QWEN_AI_PLATFORM_MESSAGE: &str = "Missing API key for Qianwen AI Platform. Set QWEN_AI_PLATFORM_API_KEY or DASHSCOPE_API_KEY environment variable or pass api_key parameter.";

fn call_for(
    provider: ProviderId,
    fixture: &EmbeddingFixture,
) -> Result<EmbeddingCall, DispatchError> {
    dispatch(provider, &fixture.context()).expect("the embedding table has this arm")
}

#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-global"),
    Some("sk-ark"),
    Some("sk-volcengine"),
    Some("sk-request")
)]
#[case::global_key(
    None,
    Some("sk-global"),
    Some("sk-ark"),
    Some("sk-volcengine"),
    Some("sk-global")
)]
#[case::ark_env(None, None, Some("sk-ark"), Some("sk-volcengine"), Some("sk-ark"))]
#[case::volcengine_env(None, None, None, Some("sk-volcengine"), Some("sk-volcengine"))]
fn volcengine_key_chain(
    #[case] request_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] ark_key: Option<&str>,
    #[case] volcengine_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = EmbeddingFixture::new();
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.environment.ark_api_key = ark_key.map(secret);
    fixture.settings.environment.volcengine_api_key = volcengine_key.map(secret);
    let call = call_for(ProviderId::Volcengine, &fixture).expect("the arm answers");
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        expected.map(str::to_owned)
    );
}

#[test]
fn volcengine_without_a_key_names_both_environment_variables() {
    let fixture = EmbeddingFixture::new();
    let outcome = dispatch(ProviderId::Volcengine, &fixture.context())
        .expect("the embedding table has this arm");
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey {
            message: VOLCENGINE_MESSAGE
        })
    ));
}

#[test]
fn volcengine_passes_extra_headers_and_an_empty_litellm_params() {
    let mut fixture = EmbeddingFixture::new();
    fixture.extra_headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.api_base = Some(String::from("https://ark.example/v1"));
    fixture.optional_params =
        serde_json::from_str(r#"{"dimensions":2}"#).expect("the fixture parses");
    let call = call_for(ProviderId::Volcengine, &fixture).expect("the arm answers");
    assert_eq!(call.headers, Some(headers(&[("x-scope", "tenant")])));
    assert_eq!(call.api_base, fixture.api_base);
    assert!(call.litellm_params.is_empty());
    assert_eq!(call.optional_params, fixture.optional_params);
}

#[test]
fn volcengine_without_extra_headers_passes_an_empty_map() {
    let fixture = EmbeddingFixture::new();
    let call = call_for(ProviderId::Volcengine, &fixture).expect("the arm answers");
    assert_eq!(call.headers, Some(headers(&[])));
}

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
    let outcome = dispatch(ProviderId::Dashscope, &fixture.context())
        .expect("the embedding table has this arm");
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey {
            message: DASHSCOPE_MESSAGE
        })
    ));
}

// the two Qwen brands read their own key before DASHSCOPE_API_KEY, and the request key before either
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
    let outcome = dispatch(ProviderId::Qwencloud, &fixture.context())
        .expect("the embedding table has this arm");
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey {
            message: QWENCLOUD_MESSAGE
        })
    ));
}

#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-brand"),
    Some("sk-dashscope"),
    Some("sk-request")
)]
#[case::brand_key(None, Some("sk-brand"), Some("sk-dashscope"), Some("sk-brand"))]
#[case::dashscope_key(None, None, Some("sk-dashscope"), Some("sk-dashscope"))]
fn qwen_ai_platform_key_chain(
    #[case] request_key: Option<&str>,
    #[case] brand_key: Option<&str>,
    #[case] dashscope_key: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = EmbeddingFixture::new();
    fixture.api_key = request_key.map(secret);
    fixture.settings.environment.qwen_ai_platform_key = brand_key.map(secret);
    fixture.settings.environment.dashscope_api_key = dashscope_key.map(secret);
    let call = call_for(ProviderId::QwenAiPlatform, &fixture).expect("the arm answers");
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        expected.map(str::to_owned)
    );
}

#[test]
fn qwen_ai_platform_without_a_key_names_its_own_variables() {
    let fixture = EmbeddingFixture::new();
    let outcome = dispatch(ProviderId::QwenAiPlatform, &fixture.context())
        .expect("the embedding table has this arm");
    assert!(matches!(
        outcome,
        Err(DispatchError::MissingKey {
            message: QWEN_AI_PLATFORM_MESSAGE
        })
    ));
}

// the GigaChat embedding chain reads the credentials env before the API key env, the reverse of its chat chain
#[rstest]
#[case::request_key(
    Some("sk-request"),
    Some("sk-global"),
    Some("sk-section"),
    Some("sk-cred"),
    Some("sk-env"),
    Some("sk-request")
)]
#[case::global_key(
    None,
    Some("sk-global"),
    Some("sk-section"),
    Some("sk-cred"),
    Some("sk-env"),
    Some("sk-global")
)]
#[case::section_key(
    None,
    None,
    Some("sk-section"),
    Some("sk-cred"),
    Some("sk-env"),
    Some("sk-section")
)]
#[case::credentials_env(None, None, None, Some("sk-cred"), Some("sk-env"), Some("sk-cred"))]
#[case::api_key_env(None, None, None, None, Some("sk-env"), Some("sk-env"))]
#[case::no_key(None, None, None, None, None, None)]
fn gigachat_embedding_key_chain(
    #[case] request_key: Option<&str>,
    #[case] global_key: Option<&str>,
    #[case] section_key: Option<&str>,
    #[case] credentials_env: Option<&str>,
    #[case] api_key_env: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let mut fixture = EmbeddingFixture::new();
    fixture.api_key = request_key.map(secret);
    fixture.settings.api_key = global_key.map(secret);
    fixture.settings.gigachat_key = section_key.map(secret);
    fixture.settings.environment.gigachat_credentials = credentials_env.map(secret);
    fixture.settings.environment.gigachat_api_key = api_key_env.map(secret);
    let call = call_for(ProviderId::Gigachat, &fixture).expect("the arm answers");
    assert_eq!(
        secret_text(call.api_key.as_ref()),
        expected.map(str::to_owned)
    );
}

#[test]
fn gigachat_embedding_drops_the_headers_and_carries_only_ssl_verify() {
    let mut fixture = EmbeddingFixture::new();
    fixture.extra_headers = Some(headers(&[("x-scope", "tenant")]));
    fixture.api_base = Some(String::from("https://gigachat.example/v1"));
    let call = call_for(ProviderId::Gigachat, &fixture).expect("the arm answers");
    assert_eq!(call.headers, None);
    assert_eq!(call.api_base, fixture.api_base);
    assert_eq!(call.litellm_params.len(), 1_usize);
    assert_eq!(call.litellm_params.get("ssl_verify"), Some(&Json::Null));
}

#[rstest]
#[case::absent(None, Json::Null)]
#[case::enabled(Some(true), Json::Bool(true))]
#[case::disabled(Some(false), Json::Bool(false))]
fn gigachat_embedding_carries_the_request_ssl_verify(
    #[case] ssl_verify: Option<bool>,
    #[case] expected: Json,
) {
    let mut fixture = EmbeddingFixture::new();
    fixture.ssl_verify = ssl_verify;
    let call = call_for(ProviderId::Gigachat, &fixture).expect("the arm answers");
    assert_eq!(call.litellm_params.get("ssl_verify"), Some(&expected));
}

#[test]
fn providers_outside_the_regional_embedding_table_have_no_arm() {
    let fixture = EmbeddingFixture::new();
    let context = fixture.context();
    assert!(dispatch(ProviderId::NlpCloud, &context).is_none());
    assert!(dispatch(ProviderId::Minimax, &context).is_none());
    assert!(dispatch(ProviderId::Modelscope, &context).is_none());
    assert!(dispatch(ProviderId::Tencent, &context).is_none());
    assert!(dispatch(ProviderId::Openai, &context).is_none());
}
