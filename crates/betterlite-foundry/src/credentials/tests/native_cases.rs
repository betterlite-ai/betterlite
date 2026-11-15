use betterlite_prompts::arguments::json::{Json, Object};

use crate::credentials::native::{foundry_rejects_tools, supports_native_responses};

use super::builders::accessors;

const V1_BASE: &str = "https://res.services.ai.azure.com/openai/v1";

pub(super) fn native_responses_eligibility() {
    base_boundaries();
    route_boundaries();
}

fn base_boundaries() {
    none_and_explicit();
    accessor_precedence();
}

fn none_and_explicit() {
    let none = accessors(None, None, None, None);
    assert!(supports_native_responses(None, None, none));
    assert!(supports_native_responses(Some("gpt-4o"), None, none));
    assert!(!supports_native_responses(
        Some("gpt-4o"),
        Some("https://foo.bar"),
        none
    ));
    assert!(supports_native_responses(
        Some("gpt-4o"),
        Some(V1_BASE),
        none
    ));
    assert!(supports_native_responses(
        Some("gpt-4o"),
        Some("https://res.openai.azure.com/openai/v1"),
        none
    ));
}

fn accessor_precedence() {
    let from_environment = accessors(None, None, Some("https://foo.bar"), None);
    assert!(!supports_native_responses(None, None, from_environment));
    let global_base = accessors(Some(V1_BASE), None, Some("https://foo.bar"), None);
    assert!(supports_native_responses(Some("gpt-4o"), None, global_base));
    assert!(!supports_native_responses(
        Some("gpt-4o"),
        Some("https://foo.bar"),
        global_base
    ));
    assert!(supports_native_responses(
        None,
        Some(V1_BASE),
        accessors(None, None, Some("https://foo.bar"), None)
    ));
}

fn route_boundaries() {
    let none = accessors(None, None, None, None);
    let rejected = [
        "azure_ai/CLAUDE-sonnet",
        "azure_ai/agents/agent-1",
        "azure_ai/model_router/gpt-4o",
        "azure_ai/model-router/gpt-4o",
        "azure_ai/MODEL_ROUTER/gpt-4o",
    ];
    for model in rejected {
        assert!(
            !supports_native_responses(Some(model), Some(V1_BASE), none),
            "{model}"
        );
    }
    let accepted = ["azure_ai/gpt-4o", "azure_ai/AGENTS/agent-1", "gpt-4o"];
    for model in accepted {
        assert!(
            supports_native_responses(Some(model), Some(V1_BASE), none),
            "{model}"
        );
    }
    assert!(!supports_native_responses(
        Some("azure_ai/agents/agent-1"),
        None,
        none
    ));
}

pub(super) fn foundry_reasoning_tool_gate() {
    absent_effort();
    supplied_effort();
}

fn absent_effort() {
    assert!(foundry_rejects_tools("gpt-6", None));
    assert!(!foundry_rejects_tools("gpt-5.6", None));
    assert!(!foundry_rejects_tools("gpt-5", None));
    assert!(foundry_rejects_tools("azure_ai/gpt-6-mini", None));
    let null = Json::Null;
    assert!(foundry_rejects_tools("gpt-6", Some(&null)));
    assert!(!foundry_rejects_tools("gpt-5.6", Some(&null)));
}

fn supplied_effort() {
    let empty_object = Json::Object(Object::default());
    let empty_string = Json::Text(String::new());
    let falsy = Json::Bool(false);
    let empty_list = Json::List(Vec::new());
    for effort in [&empty_object, &empty_string, &falsy, &empty_list] {
        assert!(foundry_rejects_tools("gpt-5.6", Some(effort)));
        assert!(!foundry_rejects_tools("gpt-5.5", Some(effort)));
        assert!(foundry_rejects_tools("gpt-6", Some(effort)));
    }
}
