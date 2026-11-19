use betterlite_models::decisions::Json;
use betterlite_openai::chat::{ChatMessage, ContentPart, MessageContent, Role};
use indexmap::IndexMap;

use crate::context_placement::facts::{CatalogModelInfo, ModelLimitKey, deployment_limit};
use crate::context_placement::request::{OutOfBand, out_of_band_text, request_byte_upper_bound};

use super::support::{deployment, json, model_catalog};

pub(super) fn run(name: &str) {
    match name {
        "out_of_band_joins_instructions_and_system" => out_of_band_joins_instructions_and_system(),
        "body_tools_win_over_request_tools"
        | "an_empty_body_tools_list_is_measured_as_declared" => {
            the_body_tools_list_is_measured_before_the_request_tools();
        }
        "declared_tools_are_measured_as_given" => declared_tools_are_measured_as_given(),
        "a_null_or_non_text_value_contributes_nothing" => {
            a_null_or_non_text_value_contributes_nothing();
        }
        "message_without_content_adds_no_bytes" | "an_empty_parts_message_adds_no_bytes" => {
            messages_without_content_add_no_bytes();
        }
        "message_parts_measure_as_their_json" => message_parts_measure_as_their_json(),
        "catalog_limits_resolve_for_a_declared_model" => {
            catalog_limits_resolve_for_a_declared_model();
        }
        "catalog_azure_resolves_by_base_model" => catalog_azure_resolves_by_base_model(),
        "catalog_unmapped_models_have_no_limit" => catalog_unmapped_models_have_no_limit(),
        "catalog_custom_provider_overrides_the_model_prefix" => {
            catalog_custom_provider_overrides_the_model_prefix();
        }
        _ => panic!("unknown helper case {name}"),
    }
}

fn out_of_band_joins_instructions_and_system() {
    let instructions = Json::Text("abc".to_owned());
    let system = Json::Text("de".to_owned());
    let out_of_band = OutOfBand {
        instructions: Some(&instructions),
        system: Some(&system),
        body_tools: None,
        request_tools: None,
    };
    assert_eq!(out_of_band_text(&out_of_band), "abcde");
}

// the body's declared list is measured as given, empty or not, and only a null body asks the request list
fn the_body_tools_list_is_measured_before_the_request_tools() {
    let body = json(r#"[{"a":1}]"#);
    let request = json(r#"[{"b":2}]"#);
    let declared = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&body),
        request_tools: Some(&request),
    };
    assert_eq!(out_of_band_text(&declared), r#"[{"a":1}]"#);
    let empty = json("[]");
    let declared_empty = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&empty),
        request_tools: Some(&request),
    };
    assert_eq!(out_of_band_text(&declared_empty), "[]");
}

fn declared_tools_are_measured_as_given() {
    let request = json(r#"[{"b":2}]"#);
    let text = Json::Text("tool".to_owned());
    let from_text = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&text),
        request_tools: Some(&request),
    };
    assert_eq!(out_of_band_text(&from_text), r#""tool""#);
    let zero = json("0");
    let from_zero = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&zero),
        request_tools: Some(&request),
    };
    assert_eq!(out_of_band_text(&from_zero), "0");
    let null = Json::Null;
    let from_null = OutOfBand {
        instructions: None,
        system: None,
        body_tools: Some(&null),
        request_tools: Some(&request),
    };
    assert_eq!(out_of_band_text(&from_null), r#"[{"b":2}]"#);
}

fn a_null_or_non_text_value_contributes_nothing() {
    let instructions = Json::Bool(true);
    let system = Json::Null;
    let out_of_band = OutOfBand {
        instructions: Some(&instructions),
        system: Some(&system),
        body_tools: None,
        request_tools: None,
    };
    assert_eq!(out_of_band_text(&out_of_band), "");
}

fn messages_without_content_add_no_bytes() {
    let bare = ChatMessage {
        role: Role::User,
        content: None,
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    };
    let empty_parts = ChatMessage {
        role: Role::User,
        content: Some(MessageContent::Parts(Vec::new())),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    };
    let out_of_band = OutOfBand::default();
    assert_eq!(
        request_byte_upper_bound(Some(&[bare, empty_parts]), &out_of_band),
        0
    );
    assert_eq!(request_byte_upper_bound(None, &out_of_band), 0);
}

fn message_parts_measure_as_their_json() {
    let mut fields = IndexMap::new();
    fields.insert("text".to_owned(), serde_json::json!("hi"));
    let part = ContentPart {
        kind: "text".to_owned(),
        fields,
    };
    let parts = ChatMessage {
        role: Role::User,
        content: Some(MessageContent::Parts(vec![part])),
        name: None,
        tool_calls: None,
        tool_call_id: None,
        passthrough: IndexMap::new(),
    };
    let out_of_band = OutOfBand::default();
    // [{"type":"text","text":"hi"}] in bytes
    assert_eq!(request_byte_upper_bound(Some(&[parts]), &out_of_band), 29);
}

fn catalog_limits_resolve_for_a_declared_model() {
    let catalog = model_catalog(
        r#"{"gpt-4o":{"litellm_provider":"openai","mode":"chat","max_input_tokens":1000,"max_output_tokens":200}}"#,
    );
    let info = CatalogModelInfo { catalog: &catalog };
    let spec = deployment("group", r#"{"model":"openai/gpt-4o"}"#, "{}");
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::InputTokens),
        Some(1000)
    );
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::OutputTokens),
        Some(200)
    );
}

fn catalog_azure_resolves_by_base_model() {
    let catalog = model_catalog(
        r#"{"azure/gpt-4o":{"litellm_provider":"azure","mode":"chat","max_input_tokens":128000}}"#,
    );
    let info = CatalogModelInfo { catalog: &catalog };
    let spec = deployment(
        "group",
        r#"{"model":"azure/my-deployment","base_model":"gpt-4o"}"#,
        "{}",
    );
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::InputTokens),
        Some(128_000)
    );
}

fn catalog_unmapped_models_have_no_limit() {
    let catalog = model_catalog(
        r#"{"gpt-4o":{"litellm_provider":"openai","mode":"chat","max_input_tokens":1000}}"#,
    );
    let info = CatalogModelInfo { catalog: &catalog };
    let unknown = deployment("group", r#"{"model":"openai/unmapped"}"#, "{}");
    assert_eq!(
        deployment_limit(&info, "group", &unknown, ModelLimitKey::InputTokens),
        None
    );
    let missing = deployment("group", "{}", "{}");
    assert_eq!(
        deployment_limit(&info, "group", &missing, ModelLimitKey::InputTokens),
        None
    );
}

fn catalog_custom_provider_overrides_the_model_prefix() {
    let catalog = model_catalog(
        r#"{"gpt-4o":{"litellm_provider":"openai","mode":"chat","max_input_tokens":1000}}"#,
    );
    let info = CatalogModelInfo { catalog: &catalog };
    let spec = deployment(
        "group",
        r#"{"model":"azure/gpt-4o","custom_llm_provider":"openai"}"#,
        "{}",
    );
    assert_eq!(
        deployment_limit(&info, "group", &spec, ModelLimitKey::InputTokens),
        Some(1000)
    );
}
