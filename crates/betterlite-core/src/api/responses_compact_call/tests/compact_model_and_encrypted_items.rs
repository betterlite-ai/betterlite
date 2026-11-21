use super::support::{
    COMPACT_OK, ENCODED_RESPONSE, Harness, Rig, base_env, body_json, call, item_input, json,
    kwargs, text_input, typed_id,
};
use crate::api::responses_compact_call::call::CompactResponsesCall;
use crate::api::responses_compact_call::{acompact_responses, compact_responses};
use crate::api::responses_compact_http::service::LogContext;

const ITEM_INPUT: &str = r#"[{"type":"message","role":"user","id":"encitem_bGl0ZWxsbTptb2RlbF9pZDpkZXBsb3ltZW50X2E7aXRlbV9pZDptc2dfYWJj","content":[{"type":"input_text","text":"hello"}],"encrypted_content":"litellm_enc:bW9kZWxfaWQ6ZGVwbG95bWVudF9h;gAAAA-secret"}]"#;
const RESTORED_BODY: &str = r#"{"model":"gpt-4o","input":[{"type":"message","role":"user","id":"msg_abc","content":[{"type":"input_text","text":"hello"}],"encrypted_content":"gAAAA-secret"}],"instructions":"short","previous_response_id":"resp_previous"}"#;

pub(super) async fn sync_and_async() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.repeat(200, COMPACT_OK, 2);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let compacting = encrypted_call();
    let sync_reply = compact_responses(&rig.deps(), &compacting, &mut log)
        .await
        .expect("the sync compact call succeeds");
    let async_reply = acompact_responses(&rig.deps(), &compacting, &mut log)
        .await
        .expect("the async compact call succeeds");

    assert_eq!(typed_id(&sync_reply), ENCODED_RESPONSE);
    assert_eq!(typed_id(&async_reply), ENCODED_RESPONSE);
    let requests = rig.wire.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].url.as_str(),
        "http://mock.test/v1/responses/compact"
    );
    assert_eq!(body_json(&requests[0]), json(RESTORED_BODY));
    assert_eq!(body_json(&requests[1]), json(RESTORED_BODY));
    assert_eq!(
        requests[0]
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some("Bearer sk-provider")
    );
}

fn encrypted_call() -> CompactResponsesCall {
    let mut compacting = call("openai/gpt-4o", item_input(ITEM_INPUT));
    compacting.instructions = Some(String::from("short"));
    compacting.previous_response_id = Some(String::from("resp_previous"));
    compacting.kwargs = kwargs(
        r#"{"litellm_metadata":{"model_info":{"id":"deployment_a"},"encrypted_content_affinity_enabled":true}}"#,
    );
    compacting
}

pub(super) async fn routing_prefix() {
    let mut harness = Harness::new();
    let rig = Rig::default();
    rig.wire.queue(200, COMPACT_OK);
    let prepost = base_env(&harness.redactor, &harness.sink, harness.now);
    let mut log = LogContext {
        state: &mut harness.state,
        env: &prepost,
    };
    let mut routed = call("responses/gpt-4o", text_input("hello"));
    routed.custom_llm_provider = Some(String::from("openai"));
    compact_responses(&rig.deps(), &routed, &mut log)
        .await
        .expect("the routed compact call succeeds");

    let requests = rig.wire.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        body_json(&requests[0]),
        json(r#"{"model":"gpt-4o","input":"hello"}"#)
    );
    let seen = rig.resolver.seen();
    let routed_seen = seen.last().expect("the routed call resolves once");
    assert_eq!(routed_seen.model, "responses/gpt-4o");
    assert_eq!(routed_seen.custom_llm_provider.as_deref(), Some("openai"));
}
