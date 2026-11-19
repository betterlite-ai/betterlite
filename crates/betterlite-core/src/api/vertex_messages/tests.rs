#![cfg(test)]

use std::future::{Future, Ready, ready};

use betterlite_anthropic::anthropic_capability_catalog::OptionalParams;
use betterlite_anthropic::anthropic_capability_catalog::thinking::ThinkingError;
use betterlite_anthropic::capabilities::reasoning::ReasoningEffortError;
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::system_placement::text_parts::CONVERTED_SYSTEM_NOTE;

use super::error::NativeVertexMessagesError;
use super::{NativeMessages, VertexMessagesOverrides, adapt};
use crate::api::messages_native_transform::capabilities::{
    LegacyShapes, Thinking, ThinkingCapabilities,
};

const MID_SYSTEM: &str = "supports_mid_conversation_system";
const MODEL: &str = "claude-sonnet-4-6";

fn json(text: &str) -> Json {
    serde_json::from_str(text).expect("the test JSON parses")
}

fn messages() -> Vec<Json> {
    vec![
        json(
            r#"{"role":"system","content":[{"type":"text","text":"x-anthropic-billing-header: abc"},{"type":"text","text":"kept"}]}"#,
        ),
        json(
            r#"{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral","scope":"global"}}]}"#,
        ),
    ]
}

fn params() -> Object {
    match json(r#"{"max_tokens":64}"#) {
        Json::Object(fields) => fields,
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) | Json::List(_) => {
            panic!("the test params are an object")
        }
    }
}

struct FakeCapabilities {
    mid_system: bool,
    asked: Vec<(String, String)>,
}

impl ThinkingCapabilities for FakeCapabilities {
    fn supports(&mut self, _model: &str, key: &str, provider: &str) -> Ready<bool> {
        self.asked.push((key.to_owned(), provider.to_owned()));
        ready(self.mid_system && key == MID_SYSTEM)
    }

    fn map_effort<'seam>(
        &'seam mut self,
        _model: &'seam str,
        _effort: &'seam str,
        _provider: &'seam str,
    ) -> Ready<Result<Option<Thinking>, ReasoningEffortError>> {
        ready(Ok(None))
    }

    fn validate_effort(
        &mut self,
        _model: &str,
        _effort: Option<&str>,
        _provider: &str,
    ) -> Ready<Option<String>> {
        ready(None)
    }

    fn supports_effort_param(&mut self, _model: &str, _provider: &str) -> Ready<bool> {
        ready(false)
    }

    fn drop_disabled_thinking(
        &mut self,
        _model: &str,
        _params: &mut OptionalParams,
        _provider: &str,
    ) -> Ready<bool> {
        ready(false)
    }

    fn translate_legacy_thinking<'seam>(
        &'seam mut self,
        _request: &'seam mut LegacyShapes<'seam>,
    ) -> Ready<Result<(), ThinkingError>> {
        ready(Ok(()))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the override failed")]
struct OverrideFailure;

#[derive(Default)]
struct RecordingOverrides {
    calls: usize,
    saw_system: bool,
    saw_scope: bool,
    saw_model: bool,
    fail: bool,
}

impl VertexMessagesOverrides for RecordingOverrides {
    type Error = OverrideFailure;

    fn apply<'body>(
        &'body mut self,
        body: &'body mut Object,
        _model: &'body str,
    ) -> impl Future<Output = Result<(), OverrideFailure>> + Send + 'body {
        self.calls += 1;
        self.saw_system = body.contains_key("system");
        self.saw_scope = carries_scope(body);
        self.saw_model = body.contains_key("model");
        body.insert(
            "anthropic_version".to_owned(),
            Json::Text("vertex-2023-10-16".to_owned()),
        );
        body.shift_remove("model");
        if self.fail {
            ready(Err(OverrideFailure))
        } else {
            ready(Ok(()))
        }
    }
}

fn carries_scope(value: &Json) -> bool {
    match value {
        Json::Object(fields) => fields.iter().any(|(key, value)| {
            (key == "cache_control"
                && value
                    .as_object()
                    .is_some_and(|control| control.contains_key("scope")))
                || carries_scope(value)
        }),
        Json::List(items) => items.iter().any(carries_scope),
        Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => false,
    }
}

fn request(model: &str, messages: Vec<Json>, optional_params: Object) -> NativeMessages<'_> {
    NativeMessages {
        model,
        messages,
        optional_params,
    }
}

#[tokio::test]
async fn native_vertex_adapter() {
    let mut capabilities = FakeCapabilities {
        mid_system: false,
        asked: Vec::new(),
    };
    let mut overrides = RecordingOverrides::default();
    let body = adapt(
        &mut capabilities,
        request(MODEL, messages(), params()),
        &mut overrides,
    )
    .await
    .expect("the adapter runs");
    // the parent hoisted the leading system turn and filtered its billing block
    assert_eq!(
        body.get("system"),
        Some(&json(r#"[{"type":"text","text":"kept"}]"#))
    );
    // the parent removed the system turn and the cache scope, leaving the user turn
    assert_eq!(
        body.get("messages"),
        Some(&json(
            r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]"#
        ))
    );
    // the Vertex override ran on the normalized body and versioned it
    assert_eq!(
        body.get("anthropic_version"),
        Some(&json(r#""vertex-2023-10-16""#))
    );
    assert!(!body.contains_key("model"));
    assert_eq!(overrides.calls, 1);
    assert!(overrides.saw_system);
    assert!(!overrides.saw_scope);
    assert!(overrides.saw_model);
    assert!(
        capabilities
            .asked
            .contains(&(MID_SYSTEM.to_owned(), "vertex_ai".to_owned()))
    );
}

fn mid_system_messages() -> Vec<Json> {
    vec![
        json(r#"{"role":"user","content":"hi"}"#),
        json(r#"{"role":"system","content":"reminder"}"#),
        json(r#"{"role":"user","content":"there"}"#),
    ]
}

fn second_message(body: &Object) -> Object {
    let Some(Json::List(messages)) = body.get("messages") else {
        panic!("the body carries messages");
    };
    messages
        .get(1)
        .and_then(Json::as_object)
        .cloned()
        .expect("the second message")
}

#[tokio::test]
async fn native_vertex_adapter_converts_mid_system_turns() {
    let mut capabilities = FakeCapabilities {
        mid_system: false,
        asked: Vec::new(),
    };
    let body = adapt(
        &mut capabilities,
        request(MODEL, mid_system_messages(), params()),
        &mut RecordingOverrides::default(),
    )
    .await
    .expect("the adapter runs");
    let second = second_message(&body);
    assert_eq!(second.get("role").and_then(Json::as_str), Some("user"));
    // the converted turn opens with the operator note and carries the original content
    let Some(Json::List(content)) = second.get("content") else {
        panic!("the converted turn carries content blocks");
    };
    assert_eq!(
        content
            .first()
            .and_then(Json::as_object)
            .and_then(|block| block.get("text")),
        Some(&Json::Text(CONVERTED_SYSTEM_NOTE.to_owned()))
    );
}

#[tokio::test]
async fn native_vertex_adapter_keeps_mid_system_turns() {
    let mut capabilities = FakeCapabilities {
        mid_system: true,
        asked: Vec::new(),
    };
    let body = adapt(
        &mut capabilities,
        request(MODEL, mid_system_messages(), params()),
        &mut RecordingOverrides::default(),
    )
    .await
    .expect("the adapter runs");
    let second = second_message(&body);
    assert_eq!(second.get("role").and_then(Json::as_str), Some("system"));
    assert_eq!(second.get("content"), Some(&json(r#""reminder""#)));
}

#[tokio::test]
async fn native_vertex_adapter_requires_max_tokens() {
    let mut capabilities = FakeCapabilities {
        mid_system: false,
        asked: Vec::new(),
    };
    let mut overrides = RecordingOverrides::default();
    let error = adapt(
        &mut capabilities,
        request(MODEL, messages(), Object::new()),
        &mut overrides,
    )
    .await
    .expect_err("the request lacks max_tokens");
    assert_eq!(error.status_code(), 400);
    assert_eq!(overrides.calls, 0);
}

#[tokio::test]
async fn native_vertex_adapter_override_failure_is_a_500() {
    let mut capabilities = FakeCapabilities {
        mid_system: false,
        asked: Vec::new(),
    };
    let mut overrides = RecordingOverrides {
        calls: 0,
        saw_system: false,
        saw_scope: false,
        saw_model: false,
        fail: true,
    };
    let error = adapt(
        &mut capabilities,
        request(MODEL, messages(), params()),
        &mut overrides,
    )
    .await
    .expect_err("the override fails");
    assert_eq!(error.status_code(), 500);
    assert!(matches!(
        error,
        NativeVertexMessagesError::Overrides(OverrideFailure)
    ));
}
