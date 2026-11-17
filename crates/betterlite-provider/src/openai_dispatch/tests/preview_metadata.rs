use betterlite_models::controls::json_value::JsonValue;
use indexmap::IndexMap;

use super::support;
use crate::openai_dispatch::call::{ChatCall, Composition};
use crate::openai_dispatch::fixtures::{FixtureAnswer, FixtureLogger};

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

async fn sent_body(composition: &Composition, call: ChatCall) -> IndexMap<String, JsonValue> {
    let answer = FixtureAnswer::new(200, support::CHAT_REPLY.as_bytes().to_vec());
    let mut logger = FixtureLogger::default();
    let _result = support::dispatch_chat(composition, call, &answer, &mut logger)
        .await
        .expect("the chat dispatch");
    support::body_fields(&answer.requests()[0].body)
}

pub async fn check() {
    let body = r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}],"metadata":{"keep":"request"}}"#;
    let nested = metadata(&[
        (
            "requester_metadata",
            JsonValue::Object(metadata(&[
                ("user_id", support::text_value("u-1")),
                ("count", support::number_value(3)),
                ("hidden_params", support::text_value("secret")),
            ])),
        ),
        ("outer", support::text_value("ignored")),
    ]);

    // preview is off by default: the call metadata never reaches the body
    let composition = support::composition();
    assert!(!composition.provider.enable_preview_features);
    let fields = sent_body(&composition, call(body, Some(nested.clone()))).await;
    assert!(!fields.contains_key("metadata"));

    // enabled: the cleaned nested map becomes the body metadata and keeps strings only
    let mut composition = support::composition();
    composition.provider.enable_preview_features = true;
    let fields = sent_body(&composition, call(body, Some(nested))).await;
    let cleaned = support::object_field(&fields, "metadata");
    assert_eq!(cleaned.len(), 1);
    assert_eq!(
        cleaned.get("user_id").expect("the string value"),
        &support::text_value("u-1")
    );

    // an empty cleaned nested map falls back to the whole metadata, strings only
    let outer_only = metadata(&[
        (
            "requester_metadata",
            JsonValue::Object(metadata(&[("count", support::number_value(2))])),
        ),
        ("team", support::text_value("t-1")),
        ("hidden_params", support::text_value("secret")),
    ]);
    let fields = sent_body(&composition, call(body, Some(outer_only))).await;
    let cleaned = support::object_field(&fields, "metadata");
    assert_eq!(cleaned.len(), 1);
    assert_eq!(
        cleaned.get("team").expect("the fallback value"),
        &support::text_value("t-1")
    );

    // nothing survives cleaning and no map was given: the body carries no metadata
    let nothing = metadata(&[("count", support::number_value(4))]);
    assert!(
        !sent_body(&composition, call(body, Some(nothing)))
            .await
            .contains_key("metadata")
    );
    assert!(
        !sent_body(&composition, call(body, None))
            .await
            .contains_key("metadata")
    );
}

// The cleaned map keeps the first sixteen string values and drops the rest.
pub async fn check_limits_to_sixteen() {
    let mut composition = support::composition();
    composition.provider.enable_preview_features = true;
    let mut fields = IndexMap::new();
    for index in 0..18 {
        fields.insert(format!("key-{index}"), support::text_value("v"));
    }
    let call = call(
        r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
        Some(metadata(&[(
            "requester_metadata",
            JsonValue::Object(fields),
        )])),
    );
    let sent = sent_body(&composition, call).await;
    let cleaned = support::object_field(&sent, "metadata");
    assert_eq!(cleaned.len(), 16);
    assert!(cleaned.contains_key("key-15"));
    assert!(!cleaned.contains_key("key-16"));
}
