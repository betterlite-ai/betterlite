use std::sync::{Arc, Mutex};

use betterlite_config::sections::openai_credentials::Settings;
use betterlite_core::responses_bridge::{
    ModelInfoError, ReasoningEffort, ResponsesBridgeCapabilities as _,
};
use betterlite_openai::openai_base_config::Object;

use super::support::{self, FakeFacts, FakeFoundry};
use crate::completion_call::bridge_facts::ModelBridgeFacts;

fn bridge<'facts>(
    facts: &'facts FakeFacts,
    foundry: &'facts FakeFoundry,
) -> ModelBridgeFacts<'facts, FakeFoundry> {
    ModelBridgeFacts {
        facts,
        foundry,
        openai: Settings::default(),
        litellm_api_base: None,
    }
}

#[test]
fn bridge_facts_answers_model_info_and_unknown_models() {
    let info: Object = serde_json::from_str(r#"{"mode":"responses"}"#).unwrap();
    let facts = FakeFacts {
        info: Some(info.clone()),
        seen: Arc::new(Mutex::new(Vec::new())),
    };
    let foundry = FakeFoundry::default();
    let facts_bridge = bridge(&facts, &foundry);
    assert_eq!(facts_bridge.model_info("gpt-5", "openai").unwrap(), info);
    let unknown = FakeFacts::default();
    let unknown_bridge = bridge(&unknown, &foundry);
    assert!(matches!(
        unknown_bridge.model_info("other", "openai"),
        Err(ModelInfoError::Unknown { model }) if model == "other"
    ));
    assert_eq!(
        support::lock(&facts.seen).clone(),
        vec![String::from("gpt-5")]
    );
    assert_eq!(
        support::lock(&unknown.seen).clone(),
        vec![String::from("other")]
    );
}

#[test]
fn bridge_facts_passes_the_foundry_questions_to_its_field() {
    let facts = FakeFacts::default();
    let foundry = FakeFoundry {
        supports: true,
        rejects: true,
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let facts_bridge = bridge(&facts, &foundry);
    assert!(
        facts_bridge.foundry_supports_responses("azure/gpt-5", Some("https://x.openai.azure.com"))
    );
    let effort = ReasoningEffort::Text(String::from("high"));
    assert!(facts_bridge.foundry_chat_rejects_tools("azure/gpt-5", Some(&effort)));
    let calls = support::lock(&foundry.calls).clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].model, "azure/gpt-5");
    assert_eq!(
        calls[0].api_base.as_deref(),
        Some("https://x.openai.azure.com")
    );
    assert!(calls[1].rejects);
}

#[test]
fn bridge_facts_resolves_the_openai_base_in_litellms_order() {
    let facts = FakeFacts::default();
    let foundry = FakeFoundry::default();
    let mut facts_bridge = bridge(&facts, &foundry);
    assert_eq!(
        facts_bridge.resolved_openai_api_base(None),
        "https://api.openai.com/v1"
    );
    assert_eq!(
        facts_bridge.resolved_openai_api_base(Some("https://proxy.test/v1")),
        "https://proxy.test/v1"
    );
    facts_bridge.litellm_api_base = Some(String::from("https://litellm.test/v1"));
    assert_eq!(
        facts_bridge.resolved_openai_api_base(None),
        "https://litellm.test/v1"
    );
    assert_eq!(
        facts_bridge.resolved_openai_api_base(Some("https://explicit.test/v1")),
        "https://explicit.test/v1"
    );
}

#[test]
fn bridge_facts_gpt_5_check_excludes_other_providers() {
    let facts = FakeFacts::default();
    let foundry = FakeFoundry::default();
    let facts_bridge = bridge(&facts, &foundry);
    assert!(facts_bridge.is_gpt_5_model("gpt-5"));
    assert!(facts_bridge.is_gpt_5_model("openai/gpt-5"));
    assert!(facts_bridge.is_gpt_5_model("gpt-6"));
    assert!(!facts_bridge.is_gpt_5_model("azure/gpt-5"));
    assert!(!facts_bridge.is_gpt_5_model("perplexity/openai/gpt-5.2"));
    assert!(!facts_bridge.is_gpt_5_model("gpt-5-chat-latest"));
    assert!(facts_bridge.is_gpt_5_search_model("gpt-5-search-api"));
    assert!(!facts_bridge.is_gpt_5_search_model("gpt-4o"));
    assert!(facts_bridge.is_gpt_5_4_plus_model("gpt-5.4"));
    assert!(!facts_bridge.is_gpt_5_4_plus_model("gpt-5.2"));
}
