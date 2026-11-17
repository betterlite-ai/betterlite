use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::response_id_codecs::response::{build_response_id, decode_response_id};

use super::super::error::ChatResponseStreamError;
use super::fixtures::{Reply, drain, driver, driver_with_metadata, field, json, kinds};

const CONTENT_HI: &str = r#"{"id":"chat_a","created":0,"choices":[{"index":0,"delta":{"content":"hi"},"finish_reason":null}]}"#;
const FINISH_STOP: &str =
    r#"{"id":"chat_a","created":0,"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
const CITATION: &str = r#"{"type":"url_citation","url_citation":{"start_index":0,"end_index":4,"url":"https://one","title":"one"}}"#;
const OTHER_CITATION: &str = r#"{"type":"url_citation","url_citation":{"start_index":5,"end_index":9,"url":"https://two","title":"two"}}"#;
const LATE_CITATION: &str = r#"{"type":"url_citation","url_citation":{"start_index":0,"end_index":2,"url":"https://late","title":"late"}}"#;

fn event_type(event: &Json) -> Json {
    field(event, "type")
}

fn annotation_events(events: &[Json]) -> Vec<Json> {
    events
        .iter()
        .filter(|event| {
            event_type(event) == Json::Text(String::from("response.output_text.annotation.added"))
        })
        .cloned()
        .collect()
}

fn citation_chunk(citations: &[&str], content: Option<&str>) -> Json {
    let listed = citations.join(",");
    let text = content.map_or_else(String::new, |text| format!(r#","content":"{text}""#));
    json(&format!(
        r#"{{"id":"chat_a","choices":[{{"index":0,"delta":{{"annotations":[{listed}]{text}}},"finish_reason":null}}]}}"#
    ))
}

#[test]
fn driver_annotation_batch_waits_for_an_idle_chunk() {
    let (mut stream, _received) = driver(
        vec![
            Ok(citation_chunk(&[CITATION], Some("hi"))),
            Ok(json(
                r#"{"id":"chat_a","choices":[{"index":0,"delta":{},"finish_reason":null}]}"#,
            )),
            Ok(json(FINISH_STOP)),
        ],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        kinds(&events),
        [
            "response.created",
            "response.in_progress",
            "response.output_item.added",
            "response.content_part.added",
            "response.output_text.delta",
            "response.output_text.annotation.added",
            "response.output_text.done",
            "response.content_part.done",
            "response.output_item.done",
            "response.completed",
        ],
        "the annotation drains on the chunk after the text that outranked it"
    );
    let annotation = field(&events[5], "annotation");
    assert_eq!(
        field(&annotation, "type"),
        Json::Text(String::from("url_citation"))
    );
    assert_eq!(
        field(&annotation, "url"),
        Json::Text(String::from("https://one"))
    );
    assert_eq!(field(&annotation, "start_index"), json("0"));
    assert_eq!(field(&annotation, "title"), Json::Text(String::from("one")));
    assert_eq!(field(&events[5], "output_index"), json("0"));
    assert_eq!(field(&events[5], "content_index"), json("0"));
    assert_eq!(field(&events[5], "annotation_index"), json("0"));
}

#[test]
fn driver_annotation_events_are_not_drained_at_eof() {
    let (mut stream, _received) = driver(
        vec![Ok(citation_chunk(&[CITATION], Some("hi")))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    assert!(
        annotation_events(&events).is_empty(),
        "no idle chunk follows, so the queued annotation never leaves the iterator"
    );
    assert_eq!(
        kinds(&events).last().map(String::as_str),
        Some("response.completed")
    );
}

#[test]
fn driver_later_annotation_batches_are_ignored() {
    let (mut stream, _received) = driver(
        vec![
            Ok(citation_chunk(&[CITATION, OTHER_CITATION], None)),
            Ok(citation_chunk(&[LATE_CITATION], None)),
            Ok(json(FINISH_STOP)),
        ],
        Reply::None,
        "{}",
        &["u1"],
    );
    let events = drain(&mut stream);
    let annotations = annotation_events(&events);
    assert_eq!(annotations.len(), 2, "only the first batch converts");
    assert_eq!(field(&annotations[0], "annotation_index"), json("0"));
    assert_eq!(
        field(&field(&annotations[1], "annotation"), "url"),
        Json::Text(String::from("https://two"))
    );
    assert_eq!(
        field(&annotations[1], "annotation_index"),
        json("1"),
        "the indexes count from zero inside the first batch"
    );
}

#[test]
fn driver_provider_fields_merge_with_the_delta_winning() {
    let (mut stream, _received) = driver(
        vec![Ok(json(
            r#"{"id":"chat_a","provider_specific_fields":{"k":"chunk","a":"1"},"choices":[{"index":0,"delta":{"provider_specific_fields":{"k":"delta"},"content":"hi"},"finish_reason":null}]}"#,
        ))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    let response = field(events.last().expect("the stream emits events"), "response");
    let surfaced = field(&response, "provider_specific_fields");
    assert_eq!(field(&surfaced, "k"), Json::Text(String::from("delta")));
    assert_eq!(field(&surfaced, "a"), Json::Text(String::from("1")));
}

#[test]
fn driver_web_search_indexes_are_reserved_before_conversion() {
    let (mut stream, _received) = driver(
        vec![
            Ok(json(
                r#"{"id":"chat_a","provider_specific_fields":{"web_search_calls":[{"id":"ws_1","status":"in_progress"}]},"choices":[{"index":0,"delta":{},"finish_reason":null}]}"#,
            )),
            Ok(json(
                r#"{"id":"chat_a","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]},"finish_reason":null}]}"#,
            )),
            Ok(json(FINISH_STOP)),
        ],
        Reply::Text,
        "{}",
        &["u1", "u2"],
    );
    let events = drain(&mut stream);
    let call = events
        .iter()
        .find(|event| {
            event_type(event) == Json::Text(String::from("response.output_item.added"))
                && field(&field(event, "item"), "type") == Json::Text(String::from("function_call"))
        })
        .expect("the tool call item is announced");
    assert_eq!(
        field(call, "output_index"),
        json("2"),
        "the search call reserved index one before the function call converted"
    );
}

#[test]
fn driver_completed_response_is_latched() {
    let (mut stream, _received) = driver(
        vec![Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    let response = field(events.last().expect("the stream emits events"), "response");
    assert_eq!(
        field(&response, "status"),
        Json::Text(String::from("completed"))
    );
    let Some(completed) = &stream.state.progress.completed_response else {
        panic!("the completed response is latched");
    };
    let Json::Text(event_id) = field(&response, "id") else {
        panic!("the completed response carries an id");
    };
    assert_eq!(completed.id, event_id);
    assert!(stream.state.progress.finished);
}

#[test]
fn driver_incomplete_status_keeps_the_completed_event() {
    let (mut stream, _received) = driver(
        vec![Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::Length,
        "{}",
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    assert_eq!(
        kinds(&events).last().map(String::as_str),
        Some("response.completed")
    );
    let response = field(events.last().expect("the stream emits events"), "response");
    assert_eq!(
        field(&response, "status"),
        Json::Text(String::from("incomplete"))
    );
    assert_eq!(
        field(&field(&response, "incomplete_details"), "reason"),
        Json::Text(String::from("max_output_tokens"))
    );
}

#[test]
fn driver_request_options_reach_the_assembly() {
    let (mut stream, _received) = driver(
        vec![Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::Text,
        r#"{"tool_choice":"required"}"#,
        &["u1", "u2", "u3"],
    );
    let events = drain(&mut stream);
    let response = field(events.last().expect("the stream emits events"), "response");
    assert_eq!(
        field(&response, "tool_choice"),
        Json::Text(String::from("required"))
    );
    assert_eq!(field(&response, "tools"), Json::List(Vec::new()));
}

#[test]
fn driver_null_assembly_ends_without_a_completed_event() {
    let (mut stream, _received) = driver(
        vec![Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::None,
        "{}",
        &["u1"],
    );
    let events = drain(&mut stream);
    assert!(
        !kinds(&events)
            .iter()
            .any(|kind| kind == "response.completed")
    );
    assert!(stream.state.progress.finished);
    assert!(stream.state.progress.completed_response.is_none());
}

fn metadata_with(model_info: Json) -> IndexMap<String, Json> {
    let mut metadata = IndexMap::new();
    metadata.insert("model_info".to_owned(), model_info);
    metadata
}

#[test]
fn driver_metadata_model_id_encodes_the_completed_identity() {
    let (mut stream, _received) = driver_with_metadata(
        vec![Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2", "u3"],
        metadata_with(json(r#"{"id":"model-1"}"#)),
    );
    let events = drain(&mut stream);
    let response = field(events.last().expect("the stream emits events"), "response");
    let Json::Text(id) = field(&response, "id") else {
        panic!("the completed response carries an id");
    };
    let decoded = decode_response_id(&id);
    assert_eq!(decoded.model_id.flatten().as_deref(), Some("model-1"));
    assert_eq!(decoded.response_id.as_deref(), Some("chat_a"));
}

#[test]
fn driver_metadata_that_turns_invalid_after_the_created_event_fails_the_completion() {
    for (model_info, expected) in [
        (json("5"), "litellm_metadata.model_info"),
        (json(r#"{"id":7}"#), "litellm_metadata.model_info.id"),
    ] {
        let (mut stream, _received) = driver_with_metadata(
            vec![Ok(json(CONTENT_HI)), Ok(json(FINISH_STOP))],
            Reply::Text,
            "{}",
            &["u1"],
            metadata_with(json(r#"{"id":"model-1"}"#)),
        );
        stream.next_event().expect("the created event builds");
        stream.next_event().expect("the in-progress event builds");
        stream.state.litellm_metadata = metadata_with(model_info);
        let mut failure = None;
        loop {
            match stream.next_event() {
                Ok(Some(_event)) => {}
                Ok(None) => break,
                Err(error) => {
                    failure = Some(error);
                    break;
                }
            }
        }
        let Some(error) = failure else {
            panic!("the metadata shape fails the completed response");
        };
        assert!(matches!(
            error,
            ChatResponseStreamError::Metadata { field } if field == expected
        ));
    }
}

#[test]
fn driver_encoded_upstream_id_is_not_encoded_twice() {
    let encoded = build_response_id(None, None, "inner");
    let chunk =
        format!(r#"{{"id":"{encoded}","choices":[{{"index":0,"delta":{{"content":"hi"}}}}]}}"#);
    let (mut stream, _received) = driver(
        vec![Ok(json(&chunk)), Ok(json(FINISH_STOP))],
        Reply::Text,
        "{}",
        &["u1", "u2"],
    );
    let events = drain(&mut stream);
    let response = field(events.last().expect("the stream emits events"), "response");
    assert_eq!(
        field(&response, "id"),
        Json::Text(encoded),
        "an id LiteLLM already encoded is kept as it is"
    );
}
