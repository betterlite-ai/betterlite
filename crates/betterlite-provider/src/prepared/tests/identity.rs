use betterlite_config::types::{LiteLlmParams, ModelDeployment, ModelInfo};
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use secrecy::{ExposeSecret as _, SecretString};

use crate::prepared::deployment::Deployment;

fn chat() -> ChatDeployment {
    ChatDeployment::resolve(
        &ModelDeployment {
            model_name: String::from("g"),
            litellm_params: LiteLlmParams {
                ssl_verify: None,
                model: Some(String::from("aiohttp_openai/up")),
                api_base: Some(String::from("http://127.0.0.1:9")),
                api_key: Some(SecretString::from("configured-key")),
                api_version: None,
                timeout: None,
                stream_timeout: None,
                max_retries: None,
                rpm: None,
                tpm: None,
                weight: None,
            },
            model_info: ModelInfo::default(),
        },
        &OpenAiDefaults::default(),
    )
    .expect("deployment")
}

#[test]
fn provider_framework_tests_behavior() {
    let deployment = Deployment::from_chat(&chat());
    assert_eq!(deployment.model, "up");
    assert_eq!(deployment.endpoint_for("embeddings").path(), "/embeddings");
    assert_eq!(
        deployment.credential().expect("credential").expose_secret(),
        "Bearer configured-key"
    );
    let mut overridden = deployment;
    overridden.authorization = Some(SecretString::from("Basic dXNlcg=="));
    assert_eq!(
        overridden
            .credential()
            .expect("basic credential")
            .expose_secret(),
        "Basic dXNlcg=="
    );
}
