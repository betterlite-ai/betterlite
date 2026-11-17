use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

use super::support;
use crate::openai_dispatch::call::{ChatCall, Composition};
use crate::openai_dispatch::fixtures::answer::FixtureAnswer;
use crate::openai_dispatch::fixtures::logger::FixtureLogger;

const BODY: &str = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"metadata":{"keep":"request"}}"#;

fn metadata(fields: &[(&str, JsonValue)]) -> IndexMap<String, JsonValue> {
    fields
        .iter()
        .map(|(name, value)| ((*name).to_owned(), value.clone()))
        .collect()
}

fn call(body: &str, metadata: Option<IndexMap<String, JsonValue>>) -> ChatCall {
    ChatCall {
        model: String::from("gpt-4o"),
        request: support::chat_request(body),
        credentials: support::credentials(
            Some("https://explicit.test/v1"),
            Some("sk-explicit"),
            None,
        ),
        headers: None,
        extra_headers: None,
        timeout: None,
        metadata,
    }
}

fn nested() -> IndexMap<String, JsonValue> {
    metadata(&[
        (
            "requester_metadata",
            JsonValue::Object(metadata(&[
                ("user_id", support::text_value("u-1")),
                ("count", support::number_value(3)),
                ("hidden_params", support::text_value("secret")),
            ])),
        ),
        ("outer", support::text_value("ignored")),
    ])
}

fn preview_composition() -> Composition {
    let mut composition = support::composition();
    composition.provider.enable_preview_features = true;
    composition
}

async fn sent_body(composition: &Composition, call: ChatCall) -> IndexMap<String, JsonValue> {
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(composition, call, &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    support::body_fields(&answer.requests()[0].body)
}

// Preview is off by default: the call metadata never reaches the body.
pub(super) async fn check_disabled() {
    let composition = support::composition();
    assert!(!composition.provider.enable_preview_features);
    let fields = sent_body(&composition, call(BODY, Some(nested()))).await;
    assert!(!fields.contains_key("metadata"));
}

// Enabled: the cleaned nested map becomes the body metadata, or the whole map when cleaning it leaves nothing.
pub(super) async fn check_cleaning() {
    let fields = sent_body(&preview_composition(), call(BODY, Some(nested()))).await;
    let JsonValue::Object(cleaned) = support::field(&fields, "metadata") else {
        panic!("the body metadata is an object");
    };
    assert_eq!(cleaned.len(), 1);
    assert_eq!(
        cleaned.get("user_id").expect("the string value"),
        &support::text_value("u-1")
    );
    let outer_only = metadata(&[
        (
            "requester_metadata",
            JsonValue::Object(metadata(&[("count", support::number_value(2))])),
        ),
        ("team", support::text_value("t-1")),
        ("hidden_params", support::text_value("secret")),
    ]);
    let outer_fields = sent_body(&preview_composition(), call(BODY, Some(outer_only))).await;
    let JsonValue::Object(outer_cleaned) = support::field(&outer_fields, "metadata") else {
        panic!("the fallback metadata is an object");
    };
    assert_eq!(outer_cleaned.len(), 1);
    assert_eq!(
        outer_cleaned.get("team").expect("the fallback value"),
        &support::text_value("t-1")
    );
}

// Nothing survives cleaning and no map was given: the body carries no metadata.
pub(super) async fn check_nothing_survives() {
    let nothing = metadata(&[("count", support::number_value(4))]);
    let cleaned = sent_body(&preview_composition(), call(BODY, Some(nothing))).await;
    assert!(!cleaned.contains_key("metadata"));
    let absent = sent_body(&preview_composition(), call(BODY, None)).await;
    assert!(!absent.contains_key("metadata"));
}

// The cleaned map keeps the first sixteen string values and drops the rest.
pub(super) async fn check_limits_to_sixteen() {
    let mut fields = IndexMap::new();
    for index in 0_u8..18 {
        fields.insert(format!("key-{index}"), support::text_value("v"));
    }
    let call = call(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        Some(metadata(&[(
            "requester_metadata",
            JsonValue::Object(fields),
        )])),
    );
    let sent = sent_body(&preview_composition(), call).await;
    let JsonValue::Object(cleaned) = support::field(&sent, "metadata") else {
        panic!("the limited metadata is an object");
    };
    assert_eq!(cleaned.len(), 16);
    assert!(cleaned.contains_key("key-15"));
    assert!(!cleaned.contains_key("key-16"));
}

pub(super) async fn check_all() {
    check_disabled().await;
    check_cleaning().await;
    check_nothing_survives().await;
    check_limits_to_sixteen().await;
}
