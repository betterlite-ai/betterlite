use betterlite_models::decisions::Json;

use crate::api::chat_response_stream_items::done::{
    create_output_content_part_done_event, create_output_item_done_event,
    create_output_text_done_event, create_reasoning_output_item_done_event,
    create_reasoning_summary_part_done_event, create_reasoning_summary_text_done_event,
};
use crate::api::chat_response_stream_items::error::StreamItemsError;

use super::fixtures::{completion, field, json, sources, state_with_details};

#[test]
fn message_done_payloads() {
    let mut state = state_with_details("{}", None, None);
    state.progress.cached_item_id = Some(String::from("msg_c"));
    state.progress.message_output_index = 2;
    let sources = sources(10, &[]);
    let text =
        create_output_text_done_event(&mut state, &sources, &completion(&json("\"hello\""), "[]"))
            .expect("the text done event builds");
    assert_eq!(
        json(&serde_json::to_string(&text).expect("the event serializes")),
        json(
            r#"{"type":"response.output_text.done","item_id":"msg_c","output_index":2,"content_index":0,"text":"hello"}"#
        )
    );
    let part = create_output_content_part_done_event(
        &mut state,
        &sources,
        &completion(&json("\"hello\""), "[]"),
    )
    .expect("the part done event builds");
    assert_eq!(
        json(&serde_json::to_string(&part).expect("the event serializes")),
        json(
            r#"{"type":"response.content_part.done","item_id":"msg_c","output_index":2,"content_index":0,"part":{"type":"output_text","text":"hello","annotations":[],"logprobs":null}}"#
        )
    );
    state.progress.litellm_model_response = Some(completion(&json("\"stored\""), "[]"));
    let item =
        create_output_item_done_event(&mut state, &sources, &completion(&json("\"passed\""), "[]"))
            .expect("the item done event builds");
    assert_eq!(
        json(&serde_json::to_string(&item).expect("the event serializes")),
        json(
            r#"{"type":"response.output_item.done","output_index":2,"sequence_number":1,"item":{"id":"msg_c","status":"completed","type":"message","role":"assistant","content":[{"type":"output_text","text":"stored","annotations":[]}]}}"#
        )
    );
    assert_eq!(
        state.progress.sequence_number, 0,
        "the done builders never advance the shared sequence"
    );
}

#[test]
fn message_done_payloads_keep_citation_annotations() {
    let annotated = json(
        r#"{"choices":[{"message":{"content":"hi","annotations":[{"type":"url_citation","url_citation":{"start_index":1,"end_index":4,"url":"https://example.test","title":"T"}},{"type":"file_citation"},{"type":"url_citation","url_citation":{}}]}}]}"#,
    );
    let mut state = state_with_details("{}", None, None);
    state.progress.cached_item_id = Some(String::from("msg_c"));
    let part = create_output_content_part_done_event(&mut state, &sources(10, &[]), &annotated)
        .expect("the part done event builds");
    assert_eq!(
        field(
            &field(
                &json(&serde_json::to_string(&part).expect("the event serializes")),
                "part"
            ),
            "annotations"
        ),
        json(
            r#"[{"type":"url_citation","start_index":1,"end_index":4,"url":"https://example.test","title":"T"},{"type":"url_citation","start_index":null,"end_index":null,"url":null,"title":null}]"#
        )
    );
    state.progress.litellm_model_response = Some(annotated);
    let item = create_output_item_done_event(&mut state, &sources(10, &[]), &Json::Null)
        .expect("the item done event builds");
    let content = field(
        &json(&serde_json::to_string(&item).expect("the event serializes")),
        "item",
    );
    let Json::List(parts) = field(&content, "content") else {
        panic!("the item content is a list");
    };
    assert_eq!(
        field(&parts[0], "annotations"),
        json(
            r#"[{"type":"url_citation","start_index":1,"end_index":4,"url":"https://example.test","title":"T"},{"type":"url_citation","start_index":null,"end_index":null,"url":null,"title":null}]"#
        )
    );
}

#[test]
fn message_done_payloads_default_missing_content_to_empty() {
    let sources = sources(10, &[]);
    for completion in [
        json(r#"{"choices":[{}]}"#),
        json(r#"{"choices":[{"message":{}}]}"#),
        json(r#"{"choices":[{"message":{"content":null}}]}"#),
        json(r#"{"choices":[{"message":{"content":""}}]}"#),
    ] {
        let mut state = state_with_details("{}", None, None);
        state.progress.cached_item_id = Some(String::from("msg_c"));
        let text = create_output_text_done_event(&mut state, &sources, &completion)
            .expect("the text done event builds");
        assert_eq!(
            field(
                &json(&serde_json::to_string(&text).expect("the event serializes")),
                "text"
            ),
            json("\"\"")
        );
    }
}

#[test]
fn message_done_payloads_need_a_first_choice() {
    let sources = sources(10, &[]);
    for completion in [
        json(r#"{"choices":[]}"#),
        json(r#"{"choices":{}}"#),
        json("null"),
    ] {
        let mut state = state_with_details("{}", None, None);
        let result = create_output_text_done_event(&mut state, &sources, &completion);
        assert!(matches!(result, Err(StreamItemsError::MissingChoice)));
    }
}

#[test]
fn message_done_payloads_reject_content_that_is_not_text() {
    let sources = sources(10, &[]);
    let mut state = state_with_details("{}", None, None);
    let result = create_output_text_done_event(
        &mut state,
        &sources,
        &json(r#"{"choices":[{"message":{"content":5}}]}"#),
    );
    assert!(matches!(result, Err(StreamItemsError::ContentNotText)));
    let result = create_output_content_part_done_event(
        &mut state,
        &sources,
        &json(r#"{"choices":[{"message":{"content":[]}}]}"#),
    );
    assert!(matches!(result, Err(StreamItemsError::ContentNotText)));
}

#[test]
fn message_done_payloads_reject_annotations_that_are_not_a_list() {
    let mut state = state_with_details("{}", None, None);
    let result = create_output_content_part_done_event(
        &mut state,
        &sources(10, &[]),
        &json(r#"{"choices":[{"message":{"content":"hi","annotations":{}}}]}"#),
    );
    assert!(matches!(result, Err(StreamItemsError::AnnotationsNotList)));
}

#[test]
fn message_done_payloads_reject_annotations_that_do_not_parse() {
    let mut state = state_with_details("{}", None, None);
    let result = create_output_content_part_done_event(
        &mut state,
        &sources(10, &[]),
        &json(r#"{"choices":[{"message":{"content":"hi","annotations":[{"url_citation":5}]}}]}"#),
    );
    assert!(matches!(result, Err(StreamItemsError::Annotation { .. })));
}

#[test]
fn message_done_payloads_need_the_stored_completion() {
    let mut state = state_with_details("{}", None, None);
    let result = create_output_item_done_event(
        &mut state,
        &sources(10, &[]),
        &completion(&json("\"x\""), "[]"),
    );
    assert!(matches!(
        result,
        Err(StreamItemsError::MissingModelResponse)
    ));
}

#[test]
fn reasoning_done_payloads() {
    let text = create_reasoning_summary_text_done_event("rs_a", "why", 7);
    assert_eq!(
        json(&serde_json::to_string(&text).expect("the event serializes")),
        json(
            r#"{"type":"response.reasoning_summary_text.done","item_id":"rs_a","output_index":0,"sequence_number":7,"summary_index":0,"text":"why"}"#
        )
    );
    let part = create_reasoning_summary_part_done_event("rs_a", "why", 7);
    assert_eq!(
        json(&serde_json::to_string(&part).expect("the event serializes")),
        json(
            r#"{"type":"response.reasoning_summary_part.done","item_id":"rs_a","output_index":0,"sequence_number":7,"summary_index":0,"part":{"type":"summary_text","text":"why"}}"#
        )
    );
    let item = create_reasoning_output_item_done_event("rs_a", "why", 7);
    assert_eq!(
        json(&serde_json::to_string(&item).expect("the event serializes")),
        json(
            r#"{"type":"response.output_item.done","output_index":0,"sequence_number":7,"item":{"id":"rs_a","type":"reasoning","summary":[{"type":"summary_text","text":"why"}]}}"#
        )
    );
}

#[test]
fn reasoning_done_payloads_keep_an_empty_summary_and_zero_sequence() {
    let text = create_reasoning_summary_text_done_event("rs_b", "", 0);
    assert_eq!(
        json(&serde_json::to_string(&text).expect("the event serializes")),
        json(
            r#"{"type":"response.reasoning_summary_text.done","item_id":"rs_b","output_index":0,"sequence_number":0,"summary_index":0,"text":""}"#
        )
    );
    let part = create_reasoning_summary_part_done_event("rs_b", "", 0);
    assert_eq!(
        field(
            &json(&serde_json::to_string(&part).expect("the event serializes")),
            "part"
        ),
        json(r#"{"type":"summary_text","text":""}"#)
    );
    let item = create_reasoning_output_item_done_event("rs_b", "", 0);
    let value = json(&serde_json::to_string(&item).expect("the event serializes"));
    assert_eq!(field(&value, "output_index"), json("0"));
    assert_eq!(field(&value, "sequence_number"), json("0"));
    assert_eq!(
        field(&field(&value, "item"), "summary"),
        json(r#"[{"type":"summary_text","text":""}]"#)
    );
}
