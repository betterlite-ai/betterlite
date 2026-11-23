use betterlite_prompts::arguments::json::{Json as PromptJson, Object};

use super::support::{
    Harness, NoInterception, Nothing, OK_RESPONSE, RecordingBridge, Rig, Seams, call, field,
    kwargs, list, number_is, object, response_json, run_create, text, values,
};
use crate::api::responses_create::convert::reencode;
use crate::api::responses_create::execute::{
    DEFAULT_MOCK_TEXT, has_file_search_tool, mock_responses_api_response,
};

pub(crate) fn literal_and_default() {
    let literal = mock_responses_api_response(Some("hi")).expect("the mock response parses");
    let value: PromptJson = reencode(&literal).expect("the mock response serializes");
    let body = object(&value);
    assert_eq!(
        text(&body, "id"),
        "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b"
    );
    assert_eq!(text(&body, "object"), "response");
    number_is(&body, "created_at", 1_741_476_542.0_f64);
    literal_fields(&body);
    message_and_usage(&body);

    let default_response =
        mock_responses_api_response(None).expect("the default mock response parses");
    let default_body: PromptJson =
        reencode(&default_response).expect("the default mock response serializes");
    let default_output = list(field(&object(&default_body), "output"));
    let default_content = list(field(&object(&default_output[0]), "content"));
    assert_eq!(
        text(&object(&default_content[0]), "text"),
        DEFAULT_MOCK_TEXT
    );
    assert_eq!(
        DEFAULT_MOCK_TEXT,
        "In a peaceful grove beneath a silver moon, a unicorn named Lumina discovered a hidden pool that reflected the stars. As she dipped her horn into the water, the pool began to shimmer, revealing a pathway to a magical realm of endless night skies. Filled with wonder, Lumina whispered a wish for all who dream to find their own hidden magic, and as she glanced back, her hoofprints sparkled like stardust."
    );
}

fn literal_fields(body: &Object) {
    assert_eq!(text(body, "status"), "completed");
    assert_eq!(text(body, "model"), "gpt-4.1-2025-04-14");
    for absent in [
        "error",
        "incomplete_details",
        "instructions",
        "max_output_tokens",
        "previous_response_id",
        "user",
    ] {
        assert_eq!(
            field(body, absent),
            &PromptJson::Null,
            "{absent} stays null"
        );
    }
    assert_eq!(field(body, "parallel_tool_calls"), &PromptJson::Bool(true));
    assert_eq!(field(body, "store"), &PromptJson::Bool(true));
    number_is(body, "temperature", 1.0_f64);
    number_is(body, "top_p", 1.0_f64);
    assert_eq!(text(body, "tool_choice"), "auto");
    assert_eq!(text(body, "truncation"), "disabled");
    assert!(list(field(body, "tools")).is_empty());
    assert!(object(field(body, "metadata")).is_empty());
    let reasoning = object(field(body, "reasoning"));
    assert_eq!(field(&reasoning, "effort"), &PromptJson::Null);
    assert_eq!(field(&reasoning, "summary"), &PromptJson::Null);
    let text_block = object(field(body, "text"));
    let format = object(field(&text_block, "format"));
    assert_eq!(text(&format, "type"), "text");
}

fn message_and_usage(body: &Object) {
    let output = list(field(body, "output"));
    assert_eq!(output.len(), 1);
    let message = object(&output[0]);
    assert_eq!(text(&message, "type"), "message");
    assert_eq!(
        text(&message, "id"),
        "msg_67ccd2bf17f0819081ff3bb2cf6508e60bb6a6b452d3795b"
    );
    assert_eq!(text(&message, "status"), "completed");
    assert_eq!(text(&message, "role"), "assistant");
    let content = list(field(&message, "content"));
    assert_eq!(content.len(), 1);
    let part = object(&content[0]);
    assert_eq!(text(&part, "type"), "output_text");
    assert_eq!(text(&part, "text"), "hi");
    assert!(list(field(&part, "annotations")).is_empty());
    let usage = object(field(body, "usage"));
    number_is(&usage, "input_tokens", 36.0_f64);
    number_is(&usage, "output_tokens", 87.0_f64);
    number_is(&usage, "total_tokens", 123.0_f64);
    let input_details = object(field(&usage, "input_tokens_details"));
    number_is(&input_details, "cached_tokens", 0.0_f64);
    assert!(object(field(&usage, "output_tokens_details")).is_empty());
}

pub(crate) async fn shortcut_and_skips() {
    let rig = Rig::new();
    let gateway = NoInterception;
    let bridge = RecordingBridge::new(&rig.events);
    let nothing = Nothing;
    let seams = Seams::new(&rig, &nothing, &gateway, &bridge);
    let mut first = call("openai/gpt-4o");
    first.kwargs = kwargs(r#"{"mock_response":"hi"}"#);
    first.stream = Some(true);
    let mut harness = Harness::new();
    let created = run_create(&seams.deps(), &first, &mut harness)
        .await
        .expect("the mock answers without a provider");
    let body = object(&response_json(&created));
    assert_eq!(
        text(&body, "id"),
        "resp_67ccd2bed1ec8190b14f964abc0542670bb6a6b452d3795b"
    );
    let output = list(field(&body, "output"));
    let content = list(field(&object(&output[0]), "content"));
    assert_eq!(text(&object(&content[0]), "text"), "hi");
    assert!(rig.wire.requests().is_empty());
    assert!(rig.events.recorded().is_empty());
    assert_eq!(rig.resolver.seen().len(), 1);

    fallthroughs(&seams, &rig, &mut harness).await;
}

async fn fallthroughs(seams: &Seams<'_>, rig: &Rig, harness: &mut Harness) {
    rig.wire.queue(200, OK_RESPONSE);
    let mut empty = call("openai/gpt-4o");
    empty.kwargs = kwargs(r#"{"mock_response":""}"#);
    let fallthrough = run_create(&seams.deps(), &empty, harness)
        .await
        .expect("an empty mock text falls through to the provider");
    let fallthrough_body = object(&response_json(&fallthrough));
    assert_ne!(text(&fallthrough_body, "id"), "resp_a");
    assert!(text(&fallthrough_body, "id").starts_with("resp_"));
    assert!(list(field(&fallthrough_body, "output")).is_empty());
    assert_eq!(rig.wire.requests().len(), 1);

    rig.wire.queue(200, OK_RESPONSE);
    let mut other = call("openai/gpt-4o");
    other.kwargs = kwargs(r#"{"mock_response":5}"#);
    run_create(&seams.deps(), &other, harness)
        .await
        .expect("a non-string mock value falls through to the provider");
    assert_eq!(rig.wire.requests().len(), 2);
}

pub(crate) fn predicate() {
    assert!(!has_file_search_tool(None));
    let none_present = values("[]");
    assert!(!has_file_search_tool(Some(&none_present)));
    let search = values(r#"[{"type":"file_search","file_search":{"vector_store_ids":["vs"]}}]"#);
    assert!(has_file_search_tool(Some(&search)));
    let other = values(r#"[{"type":"web_search"}]"#);
    assert!(!has_file_search_tool(Some(&other)));
    let scalar = values("[1]");
    assert!(!has_file_search_tool(Some(&scalar)));
    let mixed = values(r#"[1,{"type":"file_search"}]"#);
    assert!(has_file_search_tool(Some(&mixed)));
    let nested = values(r#"[{"file_search":{"type":"file_search"}}]"#);
    assert!(!has_file_search_tool(Some(&nested)));
}
