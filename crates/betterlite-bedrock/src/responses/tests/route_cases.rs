use betterlite_models::identity::provider_id::ProviderId;

use crate::model_routes::ParamMap;
use crate::responses::config::{
    BedrockResponsesConfig, CUSTOM_LLM_PROVIDER, SUPPORTS_NATIVE_FILE_SEARCH,
};

use super::support::MapFetcher;

const MODEL_COST: &str = r#"{
    "openai.served": {"supported_endpoints": ["/v1/responses", "/v1/chat/completions"]},
    "bedrock/openai.prefixed": {"supported_endpoints": ["/v1/responses"]},
    "chat_completions/openai.nested": {"supported_endpoints": ["/v1/responses"]},
    "openai.chat-only": {"supported_endpoints": ["/v1/chat/completions"]}
}"#;

fn selected(model: Option<&str>) -> bool {
    let model_cost: ParamMap = serde_json::from_str(MODEL_COST).expect("fixture model cost");
    BedrockResponsesConfig::for_model(model, &model_cost, MapFetcher::with(&[]))
        .expect("the capability lookup answers")
        .is_some()
}

pub(super) fn bedrock_responses_model_route() {
    assert!(selected(Some("openai.served")));
    assert!(selected(Some("openai.prefixed")));
    assert!(selected(Some("chat_completions/openai.served")));
    assert!(!selected(Some(
        "chat_completions/chat_completions/openai.served"
    )));
    assert!(!selected(Some("chat_completions/openai.nested")));
    assert!(!selected(Some("openai.chat-only")));
    assert!(!selected(Some("openai.unknown")));
    assert!(!selected(Some("chat_completions/")));
    assert!(!selected(Some("")));
    assert!(!selected(None));
    assert_eq!(CUSTOM_LLM_PROVIDER, ProviderId::Bedrock);
    assert!(!SUPPORTS_NATIVE_FILE_SEARCH);
}
