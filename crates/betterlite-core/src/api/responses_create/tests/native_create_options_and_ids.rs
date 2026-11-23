use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Json as PromptJson;

use super::support::{
    Harness, NoInterception, Nothing, OK_RESPONSE, RecordingBridge, Rig, Seams, body_json, call,
    field, header_values, item_input, json, kwargs, number_is, object, response_json, run_create,
    text,
};
use crate::api::response_id_codecs::encrypted::build_encrypted_item_id;
use crate::api::response_id_codecs::response::build_response_id;
use crate::api::responses_create::bridge::Created;

pub(crate) async fn options_and_ids() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut first = call("openai/gpt-4o");
    first.extra_headers = Some(BTreeMap::from([(
        String::from("x-trace"),
        String::from("b"),
    )]));
    first.kwargs = kwargs(
        r#"{"headers":{"X-Trace":"a"},"litellm_metadata":{"model_info":{"id":"deployment_a"}},"litellm_call_id":"call-42"}"#,
    );
    first.text_format = Some(json(r#"{"type":"json_object"}"#));
    first.temperature = Some(0.2_f64);
    first.service_tier = Some(String::from("flex"));
    first.safety_identifier = Some(String::from("user-a"));
    first.extra_query = Some(json(r#"{"ignored":"yes"}"#));
    let mut harness = Harness::new();
    let created = run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the native create answers");
    let requests = rig.wire.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.method, http::Method::POST);
    assert_eq!(request.url.as_str(), "http://mock.test/v1/responses");
    assert_eq!(header_values(request, "x-trace"), vec![String::from("b")]);
    let body = body_json(request);
    let fields = object(&body);
    assert_eq!(text(&fields, "model"), "gpt-4o");
    assert_eq!(
        field(&fields, "input"),
        &PromptJson::Text(String::from("hello"))
    );
    let text_block = object(field(&fields, "text"));
    assert_eq!(
        text(&object(field(&text_block, "format")), "type"),
        "json_object"
    );
    number_is(&fields, "temperature", 0.2_f64);
    assert_eq!(text(&fields, "service_tier"), "flex");
    assert_eq!(text(&fields, "safety_identifier"), "user-a");
    assert!(fields.get("user").is_none());
    assert!(fields.get("store").is_none());
    assert!(fields.get("extra_query").is_none());
    assert!(
        !serde_json::to_string(&body)
            .expect("the request body serializes")
            .contains("ignored")
    );
    let response = object(&response_json(&created));
    assert_eq!(
        text(&response, "id"),
        build_response_id(Some("openai"), Some("deployment_a"), "resp_a")
    );
    let Created::Responses(stamped) = &created else {
        panic!("the answer is a typed response");
    };
    assert_eq!(
        stamped.hidden.get("custom_llm_provider"),
        Some(&Json::Text(String::from("openai")))
    );
    assert_eq!(
        rig.configs.inputs(),
        vec![ResponsesInput::Text(String::from("hello"))]
    );
}

pub(crate) async fn encrypted_item_ids() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let encoded = build_encrypted_item_id("deployment_a", "msg_orig");
    let mut first = call("openai/gpt-4o");
    first.input = item_input(&format!(
        r#"[{{"type":"message","id":"{encoded}","role":"user","content":[{{"type":"input_text","text":"hi"}}]}}]"#
    ));
    let mut harness = Harness::new();
    run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the create with an encoded item answers");
    let inputs = rig.configs.inputs();
    assert_eq!(inputs.len(), 1);
    let ResponsesInput::Items(items) = &inputs[0] else {
        panic!("the restored input must keep its items");
    };
    assert_eq!(text(&object(&items[0].0), "id"), "msg_orig");
    assert_eq!(rig.wire.requests().len(), 1);
}
