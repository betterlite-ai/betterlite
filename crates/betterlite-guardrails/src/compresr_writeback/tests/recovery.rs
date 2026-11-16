use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use rstest::rstest;

use super::support::{
    CALL_ID, NOW, ORIGINAL, SCOPE, TOOL_CONVERSATION, apply_as_caller, content_of, conversation,
    guardrail, log_target, reply, serve,
};
use crate::compresr_recovery::hash::{content_hash, recovery_marker};
use crate::compresr_recovery::store::{Originals, StoreKey};
use crate::compresr_writeback::apply_compression;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailLogTarget;

fn text(value: &str) -> GuardrailValue {
    GuardrailValue::Text(value.to_owned())
}

fn marked(compressed: &str) -> GuardrailValue {
    text(&format!(
        "{compressed}{}",
        recovery_marker(&content_hash(ORIGINAL))
    ))
}

fn key(scope: &str) -> StoreKey {
    StoreKey::resolve(Some(CALL_ID), scope).unwrap()
}

#[tokio::test]
async fn marker_and_original_are_stored_recovery() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "");
    let output = apply_as_caller(&guardrail, conversation(TOOL_CONVERSATION, "")).await;
    assert_eq!(content_of(&output, 2), marked("short"));
    let store = guardrail.recovery.as_ref().unwrap();
    let hash = content_hash(ORIGINAL);
    assert_eq!(
        store.retrieve(Some(&key(SCOPE)), &hash, NOW),
        Some(ORIGINAL.to_owned())
    );
    assert_eq!(store.retrieve(Some(&key("scope-b")), &hash, NOW), None);
}

#[tokio::test]
async fn text_parts_keep_the_marker_recovery() {
    let rows = r#"[{"role":"user","content":"find the answer"},{"role":"tool","content":[{"type":"text","text":"long tool "},{"type":"text","text":"output one","cache_control":{"type":"ephemeral"}}]}]"#;
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "");
    let output = apply_as_caller(&guardrail, conversation(rows, "")).await;
    let GuardrailValue::Array(parts) = content_of(&output, 1) else {
        panic!("a part list stays a part list");
    };
    assert_eq!(parts.len(), 1);
    let joined = "long tool \n\noutput one";
    let expected = format!("short{}", recovery_marker(&content_hash(joined)));
    assert!(matches!(parts.first(), Some(GuardrailValue::Object(part))
            if part.get("text") == Some(&text(&expected)) && part.contains_key("cache_control")));
}

#[rstest]
#[case::no_logging_object(None)]
#[case::no_authenticated_key(Some(GuardrailLogTarget {
    litellm_params: None,
    model_call_params: Some(BTreeMap::from([(
        "litellm_call_id".to_owned(),
        GuardrailValue::Text("call-1".to_owned()),
    )])),
}))]
#[tokio::test]
async fn unscoped_request_gets_no_marker_recovery(#[case] target: Option<GuardrailLogTarget>) {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "");
    let mut caller = target;
    let output = apply_compression(
        &guardrail,
        conversation(TOOL_CONVERSATION, ""),
        &mut BTreeMap::new(),
        GuardrailInputType::Request,
        caller.as_mut(),
    )
    .await
    .unwrap();
    assert_eq!(content_of(&output, 2), text("short"));
    assert!(output.tools.is_none());
    let store = guardrail.recovery.as_ref().unwrap();
    assert_eq!(
        store.retrieve(Some(&key("")), &content_hash(ORIGINAL), NOW),
        None
    );
    assert_eq!(
        *guardrail.no_scope_warning_until.lock().unwrap(),
        NOW.saturating_add(Duration::from_mins(15))
    );
}

#[rstest]
#[case::the_first_warning(0, 100, 1000)]
#[case::before_the_next_warning(1000, 999, 1000)]
#[case::at_the_next_warning(1000, 1000, 1900)]
#[tokio::test]
async fn the_scope_warning_repeats_once_per_interval_recovery(
    #[case] warned_until: u64,
    #[case] now: u64,
    #[case] expected: u64,
) {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let mut guardrail = guardrail(&server.base, "");
    *guardrail.no_scope_warning_until.lock().unwrap() = Duration::from_secs(warned_until);
    guardrail.runtime.clock = Box::new(move || Duration::from_secs(now));
    apply_compression(
        &guardrail,
        conversation(TOOL_CONVERSATION, ""),
        &mut BTreeMap::new(),
        GuardrailInputType::Request,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        *guardrail.no_scope_warning_until.lock().unwrap(),
        Duration::from_secs(expected)
    );
}

#[tokio::test]
async fn a_request_without_a_call_id_gets_no_marker_recovery() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "");
    let mut target = log_target(SCOPE, "");
    let output = apply_compression(
        &guardrail,
        conversation(TOOL_CONVERSATION, ""),
        &mut BTreeMap::new(),
        GuardrailInputType::Request,
        Some(&mut target),
    )
    .await
    .unwrap();
    assert_eq!(content_of(&output, 2), text("short"));
    assert!(output.tools.is_none());
    assert_eq!(
        *guardrail.no_scope_warning_until.lock().unwrap(),
        Duration::ZERO
    );
}

#[tokio::test]
async fn disabled_retrieval_issues_no_marker_recovery() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "      enable_retrieval: false\n");
    let output = apply_as_caller(&guardrail, conversation(TOOL_CONVERSATION, "")).await;
    assert_eq!(content_of(&output, 2), text("short"));
    assert!(output.tools.is_none());
    assert_eq!(
        *guardrail.no_scope_warning_until.lock().unwrap(),
        Duration::ZERO
    );
}

// The original is 20 bytes; a cap of 0 turns the limit off
#[rstest]
#[case::one_byte_short(19, false)]
#[case::exactly_the_cap(20, true)]
#[case::roomy(21, true)]
#[case::unlimited(0, true)]
#[tokio::test]
async fn the_byte_cap_decides_the_marker_recovery(#[case] cap: usize, #[case] marker: bool) {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let params = format!("      max_bytes_per_call: {cap}\n");
    let guardrail = guardrail(&server.base, &params);
    let output = apply_as_caller(&guardrail, conversation(TOOL_CONVERSATION, "")).await;
    let hash = content_hash(ORIGINAL);
    let stored = guardrail
        .recovery
        .as_ref()
        .unwrap()
        .retrieve(Some(&key(SCOPE)), &hash, NOW);
    assert_eq!(marker, stored.is_some());
    assert_eq!(output.tools.is_some(), marker);
    assert_eq!(
        content_of(&output, 2),
        if marker {
            marked("short")
        } else {
            text("short")
        }
    );
}

// An earlier turn under the same key already holds bytes, and a hash already stored costs nothing more
#[rstest]
#[case::another_original("seed-hash".to_owned(), "x".repeat(15), 30, false)]
#[case::the_same_original(content_hash(ORIGINAL), ORIGINAL.to_owned(), 20, true)]
#[tokio::test]
async fn earlier_turns_count_against_the_cap_recovery(
    #[case] seeded_hash: String,
    #[case] seeded_text: String,
    #[case] cap: usize,
    #[case] marker: bool,
) {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let params = format!("      max_bytes_per_call: {cap}\n");
    let guardrail = guardrail(&server.base, &params);
    let seed = Originals::from([(seeded_hash, seeded_text)]);
    let store = guardrail.recovery.as_ref().unwrap();
    store.store(&key(SCOPE), seed, NOW, 0);
    let output = apply_as_caller(&guardrail, conversation(TOOL_CONVERSATION, "")).await;
    assert_eq!(
        content_of(&output, 2),
        if marker {
            marked("short")
        } else {
            text("short")
        }
    );
}

#[tokio::test]
async fn later_turns_add_to_the_same_call_recovery() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "");
    apply_as_caller(&guardrail, conversation(TOOL_CONVERSATION, "")).await;
    let second = TOOL_CONVERSATION.replace("long tool output one", "long tool output two");
    apply_as_caller(&guardrail, conversation(&second, "")).await;
    let store = guardrail.recovery.as_ref().unwrap();
    let second_original = "long tool output two";
    assert_eq!(
        store.retrieve(Some(&key(SCOPE)), &content_hash(ORIGINAL), NOW),
        Some(ORIGINAL.to_owned())
    );
    assert_eq!(
        store.retrieve(Some(&key(SCOPE)), &content_hash(second_original), NOW),
        Some(second_original.to_owned())
    );
}

#[tokio::test]
async fn an_empty_call_id_falls_through_to_the_next_recovery() {
    let server = serve(200, &reply(&[("short", 10, 4)])).await;
    let guardrail = guardrail(&server.base, "");
    let mut target = log_target(SCOPE, CALL_ID);
    target.model_call_params = Some(BTreeMap::from([("litellm_call_id".to_owned(), text(""))]));
    target
        .litellm_params
        .get_or_insert_with(BTreeMap::new)
        .insert("litellm_call_id".to_owned(), text(CALL_ID));
    let output = apply_compression(
        &guardrail,
        conversation(TOOL_CONVERSATION, ""),
        &mut BTreeMap::new(),
        GuardrailInputType::Request,
        Some(&mut target),
    )
    .await
    .unwrap();
    assert_eq!(content_of(&output, 2), marked("short"));
}
