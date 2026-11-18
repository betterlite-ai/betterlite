use rstest::rstest;

use super::*;

#[tokio::test]
async fn response_events_rewrite_the_id_provider_and_model() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = model_state("deployment_a");
    let created = rig
        .process(CREATED)
        .await
        .expect("the created event is processed")
        .expect("the created event is handed on");
    let expected_id = build_response_id(Some("openai"), Some("deployment_a"), "resp_1");
    let created_json = event_json(&created);
    assert_eq!(
        at(&created_json, &["response", "id"]),
        &Json::Text(expected_id.clone())
    );
    let decoded = decode_response_id(&expected_id);
    assert_eq!(decoded.custom_llm_provider, Some(Some("openai".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("deployment_a".to_owned())));
    assert_eq!(decoded.response_id, Some("resp_1".to_owned()));
    let Json::Text(container) = at(&created_json, &["response", "output", "0", "container_id"])
    else {
        panic!("the output item keeps a container id");
    };
    let decoded_container = decode_container_id(container);
    assert_eq!(
        decoded_container.custom_llm_provider,
        Some(Some("openai".to_owned()))
    );
    assert_eq!(
        decoded_container.model_id,
        Some(Some("deployment_a".to_owned()))
    );
    assert_eq!(decoded_container.response_id, Some("c".to_owned()));
    assert!(rig.state.completed_response.is_none());
    assert!(!rig.state.finished);
}

#[tokio::test]
async fn in_progress_events_rewrite_the_response_identifiers() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = model_state("deployment_a");
    let event = rig
        .process(IN_PROGRESS)
        .await
        .expect("the progress event is processed")
        .expect("the progress event is handed on");
    let json = event_json(&event);
    assert_eq!(
        at(&json, &["response", "id"]),
        &Json::Text(build_response_id(
            Some("openai"),
            Some("deployment_a"),
            "resp_1"
        ))
    );
    let Json::Text(container) = at(&json, &["response", "output", "0", "container_id"]) else {
        panic!("the in-progress item keeps a container id");
    };
    let decoded = decode_container_id(container);
    assert_eq!(decoded.custom_llm_provider, Some(Some("openai".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("deployment_a".to_owned())));
    assert_eq!(decoded.response_id, Some("c".to_owned()));
    assert!(rig.state.completed_response.is_none());
    assert!(!rig.state.finished);
}

#[tokio::test]
async fn absent_payload_response_never_rewrites_the_event() {
    let mut rig = Rig::new(Some(TransformMode::Fixed(decode(COMPLETED))));
    rig.state = model_state("deployment_a");
    let event = rig
        .process(TEXT_DELTA)
        .await
        .expect("the fixed terminal is processed")
        .expect("the fixed terminal is handed on");
    assert_eq!(
        at(&event_json(&event), &["response", "id"]),
        &Json::Text("resp_1".to_owned())
    );
    let stored = rig
        .state
        .completed_response
        .as_ref()
        .expect("the terminal is stored");
    assert_eq!(
        at(&event_json(stored), &["response", "id"]),
        &Json::Text("resp_1".to_owned())
    );
}

#[tokio::test]
async fn payload_response_on_a_typed_delta_is_rewritten() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = model_state("deployment_a");
    let payload = r#"{"type":"response.output_text.delta","item_id":"msg_1","output_index":0,"content_index":0,"delta":"hi","response":{"id":"resp_1","created_at":1,"output":[{"type":"message","id":"msg_1","container_id":"c"}]}}"#;
    let event = rig
        .process(payload)
        .await
        .expect("the delta is processed")
        .expect("the delta is handed on");
    let json = event_json(&event);
    assert_eq!(
        at(&json, &["response", "id"]),
        &Json::Text(build_response_id(
            Some("openai"),
            Some("deployment_a"),
            "resp_1"
        ))
    );
    assert_eq!(at(&json, &["delta"]), &Json::Text("hi".to_owned()));
    assert_eq!(rig.state.generated_content, "hi");
}

#[rstest]
#[case::added(ITEM_ADDED)]
#[case::done(ITEM_DONE)]
#[case::generic(GENERIC_ITEM_ADDED)]
#[tokio::test]
async fn output_item_events_encode_item_containers(#[case] payload: &str) {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = model_state("deployment_a");
    let event = rig
        .process(payload)
        .await
        .expect("the item event is processed")
        .expect("the item event is handed on");
    let Json::Text(container) = at(&event_json(&event), &["item", "container_id"]) else {
        panic!("the item keeps a container id");
    };
    let decoded = decode_container_id(container);
    assert_eq!(decoded.custom_llm_provider, Some(Some("openai".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("deployment_a".to_owned())));
    assert_eq!(decoded.response_id, Some("c".to_owned()));
}

#[rstest]
#[case::typed(ANNOTATION)]
#[case::generic(GENERIC_ANNOTATION)]
#[tokio::test]
async fn annotation_events_encode_annotation_containers(#[case] payload: &str) {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = model_state("deployment_a");
    let event = rig
        .process(payload)
        .await
        .expect("the annotation is processed")
        .expect("the annotation is handed on");
    let Json::Text(container) = at(&event_json(&event), &["annotation", "container_id"]) else {
        panic!("the annotation keeps a container id");
    };
    let decoded = decode_container_id(container);
    assert_eq!(decoded.custom_llm_provider, Some(Some("openai".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("deployment_a".to_owned())));
    assert_eq!(decoded.response_id, Some("c".to_owned()));
}

#[rstest]
#[case::typed(PART_TYPED)]
#[case::dict(PART_DICT)]
#[tokio::test]
async fn part_events_encode_annotation_containers(#[case] payload: &str) {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = model_state("deployment_a");
    let event = rig
        .process(payload)
        .await
        .expect("the part is processed")
        .expect("the part is handed on");
    let Json::Text(container) = at(
        &event_json(&event),
        &["part", "annotations", "0", "container_id"],
    ) else {
        panic!("the part keeps a container id");
    };
    let decoded = decode_container_id(container);
    assert_eq!(decoded.custom_llm_provider, Some(Some("openai".to_owned())));
    assert_eq!(decoded.model_id, Some(Some("deployment_a".to_owned())));
    assert_eq!(decoded.response_id, Some("c".to_owned()));
}

#[tokio::test]
async fn refusal_parts_leave_their_payload_unchanged() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = model_state("deployment_a");
    let event = rig
        .process(PART_REFUSAL)
        .await
        .expect("the refusal part is processed")
        .expect("the refusal part is handed on");
    assert_eq!(
        at(&event_json(&event), &["part", "refusal"]),
        &Json::Text("no".to_owned())
    );
}

#[tokio::test]
async fn unrelated_events_pass_through_unchanged() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    let event = rig
        .process(UNRELATED)
        .await
        .expect("the unrelated event is processed")
        .expect("the unrelated event is handed on");
    let parsed: Json = serde_json::from_str(UNRELATED).expect("the fixture parses");
    assert_eq!(event_json(&event), parsed);
}

#[rstest]
#[case::added(CIPHER_ADDED)]
#[case::done(CIPHER_DONE)]
#[tokio::test]
async fn affinity_wraps_added_and_done_cipher_items(#[case] payload: &str) {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = affine_state(true, Some("deployment_a"));
    let event = rig
        .process(payload)
        .await
        .expect("the cipher item is processed")
        .expect("the cipher item is handed on");
    let Json::Text(wrapped) = at(&event_json(&event), &["item", "encrypted_content"]) else {
        panic!("the item keeps its cipher");
    };
    assert_eq!(wrapped, &wrap_encrypted_content("cipher", "deployment_a"));
    assert_eq!(
        unwrap_encrypted_content(wrapped),
        (Some("deployment_a".to_owned()), "cipher".to_owned())
    );
}

#[rstest]
#[case::disabled(CIPHER_ADDED, false, Some("deployment_a"))]
#[case::missing_model(CIPHER_ADDED, true, None)]
#[case::empty_model(CIPHER_ADDED, true, Some(""))]
#[case::empty_cipher(CIPHER_EMPTY, true, Some("deployment_a"))]
#[case::numeric_cipher(CIPHER_NUMBER, true, Some("deployment_a"))]
#[case::generic_item(CIPHER_GENERIC_ITEM, true, Some("deployment_a"))]
#[case::item_on_delta(CIPHER_ON_DELTA, true, Some("deployment_a"))]
#[tokio::test]
async fn affinity_guards_leave_the_cipher_untouched(
    #[case] payload: &str,
    #[case] enabled: bool,
    #[case] model_id: Option<&str>,
) {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = affine_state(enabled, model_id);
    let event = rig
        .process(payload)
        .await
        .expect("the guarded event is processed")
        .expect("the guarded event is handed on");
    let parsed: Json = serde_json::from_str(payload).expect("the fixture parses");
    assert_eq!(
        at(&event_json(&event), &["item"]),
        at(&parsed, &["item"]),
        "payload {payload} keeps its cipher"
    );
}

#[tokio::test]
async fn affinity_keeps_an_existing_wrapper() {
    let mut rig = Rig::new(Some(TransformMode::Direct));
    rig.state = affine_state(true, Some("deployment_a"));
    let event = rig
        .process(CIPHER_ALREADY_WRAPPED)
        .await
        .expect("the wrapped item is processed")
        .expect("the wrapped item is handed on");
    let Json::Text(wrapped) = at(&event_json(&event), &["item", "encrypted_content"]) else {
        panic!("the item keeps its cipher");
    };
    let existing = "litellm_enc:bW9kZWxfaWQ6ZGVwbG95bWVudF9i;cipher";
    assert_eq!(
        unwrap_encrypted_content(wrapped),
        (Some("deployment_a".to_owned()), existing.to_owned())
    );
}
