use std::collections::BTreeMap;

use betterlite_config::loader::ConfigLoader;
use betterlite_models::controls::control_options::ControlOptions;
use betterlite_openai::chat::ChatRequest;
use betterlite_provider::operation::{ChatOperation, OperationRequest};
use betterlite_provider::prepared::PreparedRequest;
use betterlite_provider::prepared::deployment::Deployment;
use betterlite_provider::provider::Provider;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rstest::rstest;

use crate::server::chat_setup::chat_executor;

#[rstest]
#[case::aiohttp_without_a_base_or_key(
    "aiohttp_openai/up",
    "https://api.openai.com/chat/completions",
    "Bearer None"
)]
#[case::aiohttp_custom_provider_without_a_base_or_key(
    "up, custom_llm_provider: aiohttp_openai",
    "https://api.openai.com/chat/completions",
    "Bearer None"
)]
fn provider_framework_tests_behavior(
    #[case] model: &str,
    #[case] url: &str,
    #[case] authorization: &str,
) {
    let source = format!(
        "model_list:\n  - {{model_name: g, litellm_params: {{model: {model}}}, model_info: {{id: dep-1}}}}\n"
    );
    let environment = BTreeMap::from([
        ("OPENAI_API_KEY".to_owned(), "env-key".to_owned()),
        (
            "OPENAI_BASE_URL".to_owned(),
            "http://127.0.0.1:1".to_owned(),
        ),
    ]);
    let config = ConfigLoader {
        lookup: environment.clone(),
    }
    .load(&source)
    .unwrap();
    let executor = chat_executor(&config, &environment, StdRng::seed_from_u64(3)).unwrap();
    let resolved = executor.deployments.first().unwrap();
    let provider = Provider::from_deployment(resolved).unwrap();
    let inputs = Deployment::from_chat(resolved);
    let operation = OperationRequest::Chat(Box::new(ChatOperation {
        model: inputs.model.clone(),
        request: ChatRequest::parse(br#"{"model":"g","messages":[]}"#).unwrap(),
        control: ControlOptions::default(),
    }));
    let PreparedRequest::Chat(chat) = provider
        .prepare(&operation, &inputs, &executor.catalog)
        .unwrap()
    else {
        panic!("a buffered chat prepares the chat request");
    };
    assert_eq!(chat.http.url.as_str(), url);
    assert_eq!(
        chat.http.headers.get("authorization").unwrap(),
        authorization
    );
}
