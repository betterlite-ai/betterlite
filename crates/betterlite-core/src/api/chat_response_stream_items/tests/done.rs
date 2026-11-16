use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::api::chat_response_stream_items::done::{
    MessageDone, ReasoningDone, message_done_event, reasoning_done_event,
};
use crate::api::chat_response_stream_items::error::StreamItemsError;

use super::fixtures::{completion, field, json, sources, state_with_details};

#[test]
fn message_done_payloads() {
    let mut state = state_with_details("{}", None, None);
    state.progress.cached_item_id = Some(String::from("msg_c"));
    state.progress.message_output_index = 2;
    let sources = sources(10, &[]);
    let text = message_done_event(
        &mut state,
        &sources,
        &completion(&json("\"hello\""), "[]"),
        MessageDone::Text,
    )
    .expect("the text done event builds");
    assert_eq!(
        json(&serde_json::to_string(&text).expect("the event serializes")),
        json(
            r#"{"type":"response.output_text.done","item_id":"msg_c","output_index":2,"content_index":0,"text":"hello"}"#
        )
    );
    let part = message_done_event(
        &mut state,
        &sources,
        &completion(&json("\"hello\""), "[]"),
        MessageDone::ContentPart,
    )
    .expect("the part done event builds");
    assert_eq!(
        json(&serde_json::to_string(&part).expect("the event serializes")),
        json(
            r#"{"type":"response.content_part.done","item_id":"msg_c","output_index":2,"content_index":0,"part":{"type":"output_text","text":"hello","annotations":[],"logprobs":null}}"#
        )
    );
    state.progress.litellm_model_response = Some(completion(&json("\"stored\""), "[]"));
    let item = message_done_event(
        &mut state,
        &sources,
        &completion(&json("\"passed\""), "[]"),
        MessageDone::Item,
    )
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
    let part = message_done_event(
        &mut state,
        &sources(10, &[]),
        &annotated,
        MessageDone::ContentPart,
    )
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
    let item = message_done_event(
        &mut state,
        &sources(10, &[]),
        &Json::Null,
        MessageDone::Item,
    )
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

#[rstest]
#[case::no_content(r#"{"choices":[{}]}"#)]
#[case::empty_message(r#"{"choices":[{"message":{}}]}"#)]
#[case::null_content(r#"{"choices":[{"message":{"content":null}}]}"#)]
#[case::empty_content(r#"{"choices":[{"message":{"content":""}}]}"#)]
fn message_done_payloads_default_missing_content_to_empty(#[case] text: &str) {
    let mut state = state_with_details("{}", None, None);
    state.progress.cached_item_id = Some(String::from("msg_c"));
    let event = message_done_event(
        &mut state,
        &sources(10, &[]),
        &json(text),
        MessageDone::Text,
    )
    .expect("the text done event builds");
    assert_eq!(
        field(
            &json(&serde_json::to_string(&event).expect("the event serializes")),
            "text"
        ),
        json("\"\"")
    );
}

#[rstest]
#[case::no_choices(r#"{"choices":[]}"#)]
#[case::choices_object(r#"{"choices":{}}"#)]
#[case::null_completion("null")]
fn message_done_payloads_need_a_first_choice(#[case] text: &str) {
    let mut state = state_with_details("{}", None, None);
    let result = message_done_event(
        &mut state,
        &sources(10, &[]),
        &json(text),
        MessageDone::Text,
    );
    assert!(matches!(result, Err(StreamItemsError::MissingChoice)));
}

#[test]
fn message_done_payloads_reject_content_that_is_not_text() {
    let sources = sources(10, &[]);
    let mut state = state_with_details("{}", None, None);
    let result = message_done_event(
        &mut state,
        &sources,
        &json(r#"{"choices":[{"message":{"content":5}}]}"#),
        MessageDone::Text,
    );
    assert!(matches!(result, Err(StreamItemsError::ContentNotText)));
    let result = message_done_event(
        &mut state,
        &sources,
        &json(r#"{"choices":[{"message":{"content":[]}}]}"#),
        MessageDone::ContentPart,
    );
    assert!(matches!(result, Err(StreamItemsError::ContentNotText)));
}

#[test]
fn message_done_payloads_reject_annotations_that_are_not_a_list() {
    let mut state = state_with_details("{}", None, None);
    let result = message_done_event(
        &mut state,
        &sources(10, &[]),
        &json(r#"{"choices":[{"message":{"content":"hi","annotations":{}}}]}"#),
        MessageDone::ContentPart,
    );
    assert!(matches!(result, Err(StreamItemsError::AnnotationsNotList)));
}

#[test]
fn message_done_payloads_reject_annotations_that_do_not_parse() {
    let mut state = state_with_details("{}", None, None);
    let result = message_done_event(
        &mut state,
        &sources(10, &[]),
        &json(r#"{"choices":[{"message":{"content":"hi","annotations":[{"url_citation":5}]}}]}"#),
        MessageDone::ContentPart,
    );
    assert!(matches!(result, Err(StreamItemsError::Annotation { .. })));
}

#[test]
fn message_done_payloads_need_the_stored_completion() {
    let mut state = state_with_details("{}", None, None);
    let result = message_done_event(
        &mut state,
        &sources(10, &[]),
        &completion(&json("\"x\""), "[]"),
        MessageDone::Item,
    );
    assert!(matches!(
        result,
        Err(StreamItemsError::MissingModelResponse)
    ));
}

#[test]
fn reasoning_done_payloads() {
    let text = reasoning_done_event("rs_a", "why", 7, ReasoningDone::Text);
    assert_eq!(
        json(&serde_json::to_string(&text).expect("the event serializes")),
        json(
            r#"{"type":"response.reasoning_summary_text.done","item_id":"rs_a","output_index":0,"sequence_number":7,"summary_index":0,"text":"why"}"#
        )
    );
    let part = reasoning_done_event("rs_a", "why", 7, ReasoningDone::Part);
    assert_eq!(
        json(&serde_json::to_string(&part).expect("the event serializes")),
        json(
            r#"{"type":"response.reasoning_summary_part.done","item_id":"rs_a","output_index":0,"sequence_number":7,"summary_index":0,"part":{"type":"summary_text","text":"why"}}"#
        )
    );
    let item = reasoning_done_event("rs_a", "why", 7, ReasoningDone::Item);
    assert_eq!(
        json(&serde_json::to_string(&item).expect("the event serializes")),
        json(
            r#"{"type":"response.output_item.done","output_index":0,"sequence_number":7,"item":{"id":"rs_a","type":"reasoning","summary":[{"type":"summary_text","text":"why"}]}}"#
        )
    );
}

#[test]
fn reasoning_done_payloads_keep_an_empty_summary_and_zero_sequence() {
    let text = reasoning_done_event("rs_b", "", 0, ReasoningDone::Text);
    assert_eq!(
        json(&serde_json::to_string(&text).expect("the event serializes")),
        json(
            r#"{"type":"response.reasoning_summary_text.done","item_id":"rs_b","output_index":0,"sequence_number":0,"summary_index":0,"text":""}"#
        )
    );
    let part = reasoning_done_event("rs_b", "", 0, ReasoningDone::Part);
    assert_eq!(
        field(
            &json(&serde_json::to_string(&part).expect("the event serializes")),
            "part"
        ),
        json(r#"{"type":"summary_text","text":""}"#)
    );
    let item = reasoning_done_event("rs_b", "", 0, ReasoningDone::Item);
    let value = json(&serde_json::to_string(&item).expect("the event serializes"));
    assert_eq!(field(&value, "output_index"), json("0"));
    assert_eq!(field(&value, "sequence_number"), json("0"));
    assert_eq!(
        field(&field(&value, "item"), "summary"),
        json(r#"[{"type":"summary_text","text":""}]"#)
    );
}
