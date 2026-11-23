use std::collections::BTreeMap;

use betterlite_models::controls::json_value::JsonValue;
use betterlite_models::decisions::Json;
use betterlite_openai::response_input::ResponsesInput;
use betterlite_prompts::arguments::json::Json as PromptJson;
use secrecy::SecretString;

use super::builders::{
    body_json, call, failure, field, header_values, item_input, json, kwargs, number_is, object,
    prompt_json, response_json, run_create, text, values,
};
use super::support::{
    Harness, NoInterception, Nothing, OK_RESPONSE, RecordingBridge, Rig, Seams, Wire,
};
use crate::api::response_id_codecs::encrypted::build_encrypted_item_id;
use crate::api::response_id_codecs::response::build_response_id;
use crate::api::responses_compact_http::service::{ClientKind, SuppliedClient};
use crate::api::responses_create::bridge::Created;
use crate::api::responses_create::call::input_value;

pub(crate) async fn options_and_ids() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let created = first_request(&seams).await;
    assert_options(&rig);
    assert_ids(&rig, &created);
    text_forms(&rig, &seams).await;
    client_header_only(&rig, &seams).await;
    credential_precedence().await;
    input_values();
}

async fn first_request(seams: &Seams<'_>) -> Created<()> {
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
    run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the native create answers")
}

fn assert_options(rig: &Rig) {
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
}

fn assert_ids(rig: &Rig, created: &Created<()>) {
    let response = object(&response_json(created));
    assert_eq!(
        text(&response, "id"),
        build_response_id(Some("openai"), Some("deployment_a"), "resp_a")
    );
    let Created::Responses(stamped) = created else {
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
async fn text_forms(rig: &Rig, seams: &Seams<'_>) {
    rig.wire.queue(200, OK_RESPONSE);
    let mut schema = call("openai/gpt-4o");
    schema.text_format = Some(json(
        r#"{"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object"},"strict":true}}"#,
    ));
    let mut harness = Harness::new();
    run_create(&seams.deps(), &schema, &mut harness)
        .await
        .expect("the json schema format answers");
    let schema_requests = rig.wire.requests();
    let schema_body = body_json(&schema_requests[1]);
    let schema_fields = object(&schema_body);
    let schema_text = object(field(&schema_fields, "text"));
    let format = object(field(&schema_text, "format"));
    assert_eq!(text(&format, "type"), "json_schema");
    assert_eq!(text(&format, "name"), "answer");
    assert_eq!(field(&format, "strict"), &PromptJson::Bool(true));
    assert_eq!(text(&object(field(&format, "schema")), "type"), "object");
    assert!(format.get("json_schema").is_none());

    rig.wire.queue(200, OK_RESPONSE);
    let mut given = call("openai/gpt-4o");
    given.text = Some(json(r#"{"format":{"type":"text"}}"#));
    given.text_format = Some(json(
        r#"{"type":"json_schema","json_schema":{"name":"answer","schema":{"type":"object"},"strict":true}}"#,
    ));
    run_create(&seams.deps(), &given, &mut harness)
        .await
        .expect("the given text wins over the format");
    let given_requests = rig.wire.requests();
    let given_body = body_json(&given_requests[2]);
    let given_fields = object(&given_body);
    let given_text = object(field(&given_fields, "text"));
    let given_format = object(field(&given_text, "format"));
    assert_eq!(text(&given_format, "type"), "text");
    assert!(given_format.get("name").is_none());
}
async fn client_header_only(rig: &Rig, seams: &Seams<'_>) {
    rig.wire.queue(200, OK_RESPONSE);
    let mut headers = call("openai/gpt-4o");
    headers.kwargs = kwargs(r#"{"headers":{"X-Trace":"a"}}"#);
    let mut harness = Harness::new();
    run_create(&seams.deps(), &headers, &mut harness)
        .await
        .expect("the client headers reach the request");
    let header_requests = rig.wire.requests();
    let request = &header_requests[3];
    assert_eq!(header_values(request, "x-trace"), vec![String::from("a")]);
    assert!(header_values(request, "xyzzy").is_empty());
    assert!(header_values(request, "").is_empty());
}
async fn credential_precedence() {
    let mut rig = Rig::new();
    rig.resolver.deployment_api_key = Some(SecretString::from("sk-deployment"));
    rig.resolver.deployment_api_base = Some(String::from("http://deployment.test/v1"));
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut request = call("openai/gpt-4o");
    request.api_key = Some(SecretString::from("sk-call"));
    request.api_base = Some(String::from("http://call.test/v1"));
    let mut harness = Harness::new();
    run_create(&seams.deps(), &request, &mut harness)
        .await
        .expect("the caller's endpoint answers");
    let credential_requests = rig.wire.requests();
    assert_eq!(
        credential_requests[0].url.as_str(),
        "http://call.test/v1/responses"
    );
    assert_eq!(
        header_values(&credential_requests[0], "authorization"),
        vec![String::from("Bearer sk-call")]
    );
}
fn input_values() {
    assert_eq!(
        input_value(&ResponsesInput::Text(String::from("hello"))),
        JsonValue::Text(String::from("hello"))
    );
    let listed = item_input(r#"[{"role":"user","content":"hi"}]"#);
    assert_eq!(
        input_value(&listed),
        JsonValue::Array(values(r#"[{"role":"user","content":"hi"}]"#))
    );
}

pub(crate) async fn allowed_params_and_unsupported() {
    let mut rig = Rig::new();
    rig.configs.supported = Some(vec![String::from("temperature")]);
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut allowed = call("openai/gpt-4o");
    allowed.service_tier = Some(String::from("flex"));
    allowed.allowed_openai_params = Some(vec![String::from("service_tier")]);
    let mut harness = Harness::new();
    run_create(&seams.deps(), &allowed, &mut harness)
        .await
        .expect("an allowed openai param passes the supported check");
    let requests = rig.wire.requests();
    let body = object(&body_json(&requests[0]));
    assert_eq!(text(&body, "service_tier"), "flex");

    let mut refused = call("openai/gpt-4o");
    refused.service_tier = Some(String::from("flex"));
    let error = run_create(&seams.deps(), &refused, &mut harness)
        .await
        .err()
        .expect("an unsupported param fails without drop_params");
    let rejected = failure(&error);
    assert_eq!(rejected.class, "UnsupportedParamsError");
    assert_eq!(rejected.status, 400);
    assert_eq!(rejected.kind, None);
    assert_eq!(rejected.param, None);
    assert_eq!(rejected.code, None);
    assert_eq!(rig.wire.requests().len(), 1);
}

pub(crate) async fn supplied_client_transport() {
    let rig = Rig::new();
    let supplied = Wire::new(&rig.events);
    supplied.queue(200, OK_RESPONSE);
    let client = SuppliedClient {
        kind: ClientKind::Sync,
        transport: &supplied,
    };
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut deps = seams.deps();
    deps.supplied = Some(client);
    let mut harness = Harness::new();
    run_create(&deps, &call("openai/gpt-4o"), &mut harness)
        .await
        .expect("the supplied client carries the request");
    assert_eq!(supplied.requests().len(), 1);
    assert!(rig.wire.requests().is_empty());
    assert!(rig.source.created().is_empty());
}

pub(crate) async fn extra_body() {
    let rig = Rig::new();
    rig.wire.queue(200, OK_RESPONSE);
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut request = call("openai/gpt-4o");
    request.extra_body = Some(object(&prompt_json(r#"{"x":1}"#)));
    let mut harness = Harness::new();
    run_create(&seams.deps(), &request, &mut harness)
        .await
        .expect("the extra body reaches the native request");
    let body = object(&body_json(&rig.wire.requests()[0]));
    number_is(&body, "x", 1.0_f64);
}
