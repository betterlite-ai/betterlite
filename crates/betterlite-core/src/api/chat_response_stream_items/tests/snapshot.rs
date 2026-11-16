use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::chat_response_stream_items::error::StreamItemsError;
use crate::api::chat_response_stream_items::snapshot::{
    create_litellm_model_response, merge_hidden_provider_fields, merge_provider_specific_fields,
    snapshot_chunk,
};

use super::fixtures::{
    AssemblyResponse, FixtureAssembly, field, json, state_with_assembly, state_with_details,
};

fn chunk(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the chunk is an object")
}

fn provider(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("the fields are an object")
}

fn first_content(assembled: &Json) -> Json {
    let Json::List(choices) = field(assembled, "choices") else {
        panic!("the assembled response has choices");
    };
    field(&field(&choices[0], "message"), "content")
}

#[test]
fn snapshot_and_metadata() {
    let filled = chunk(
        r#"{"id":"chatcmpl-1","usage":null,"_hidden_params":{"usage":{"prompt_tokens":3},"additional_headers":{"x-test":"one"}}}"#,
    );
    assert_eq!(
        snapshot_chunk(&filled),
        json(
            r#"{"id":"chatcmpl-1","usage":{"prompt_tokens":3},"_hidden_params":{"usage":{"prompt_tokens":3},"additional_headers":{"x-test":"one"}}}"#
        )
    );
    let existing =
        chunk(r#"{"usage":{"prompt_tokens":1},"_hidden_params":{"usage":{"prompt_tokens":9}}}"#);
    assert_eq!(
        snapshot_chunk(&existing),
        json(r#"{"usage":{"prompt_tokens":1},"_hidden_params":{"usage":{"prompt_tokens":9}}}"#)
    );
    let null_usage = chunk(r#"{"id":"c","_hidden_params":{"usage":null}}"#);
    assert_eq!(
        snapshot_chunk(&null_usage),
        json(r#"{"id":"c","_hidden_params":{"usage":null}}"#)
    );
    let plain = chunk(r#"{"id":"c","choices":[]}"#);
    assert_eq!(
        snapshot_chunk(&plain),
        json(r#"{"id":"c","choices":[],"_hidden_params":{}}"#)
    );
    let explicit_null = chunk(r#"{"id":"c","_hidden_params":null}"#);
    assert_eq!(snapshot_chunk(&explicit_null), json(r#"{"id":"c"}"#));
    let text_hidden = chunk(r#"{"id":"c","_hidden_params":"x"}"#);
    assert_eq!(
        snapshot_chunk(&text_hidden),
        json(r#"{"id":"c","_hidden_params":"x"}"#)
    );
}

#[test]
fn snapshot_and_metadata_merge_provider_fields() {
    let mut state = state_with_details("{}", None, None);
    merge_provider_specific_fields(&mut state, &provider(r#"{"web_search_results":["a"]}"#));
    merge_provider_specific_fields(
        &mut state,
        &provider(r#"{"web_search_results":["a","b"],"tool_results":["t"]}"#),
    );
    assert_eq!(
        Json::Object(state.progress.provider_specific_fields.clone()),
        json(r#"{"web_search_results":["a","b"],"tool_results":["t"]}"#)
    );
}

#[test]
fn snapshot_and_metadata_assemble_with_the_logging_context() {
    let mut state = state_with_assembly("{}", FixtureAssembly::new(AssemblyResponse::Chat));
    let snapshot =
        chunk(r#"{"id":"chatcmpl-1","choices":[{"index":0,"delta":{"content":"hello"}}]}"#);
    state.progress.chunk_snapshots.push(snapshot.clone());
    state.progress.provider_specific_fields = provider(r#"{"web_search_results":["a"]}"#);
    let assembled = create_litellm_model_response(&state)
        .expect("the assembly succeeds")
        .expect("a response comes back");
    assert_eq!(
        first_content(&assembled),
        Json::Text(String::from("stored"))
    );
    assert_eq!(
        field(
            &field(
                &field(&assembled, "hidden_params"),
                "provider_specific_fields"
            ),
            "web_search_results"
        ),
        json(r#"["a"]"#)
    );
    assert_eq!(
        state
            .logging_context
            .received
            .lock()
            .expect("the calls lock")
            .clone(),
        vec![vec![snapshot]],
        "the logging context assembles the collected snapshots"
    );
}

#[test]
fn snapshot_and_metadata_keep_a_null_assembly_null() {
    let state = state_with_assembly("{}", FixtureAssembly::new(AssemblyResponse::None));
    let assembled = create_litellm_model_response(&state).expect("the null assembly succeeds");
    assert!(assembled.is_none());
}

#[test]
fn snapshot_and_metadata_create_the_hidden_map_for_text_responses() {
    let mut state = state_with_assembly("{}", FixtureAssembly::new(AssemblyResponse::Text));
    state.progress.provider_specific_fields = provider(r#"{"tool_results":["t"]}"#);
    let assembled = create_litellm_model_response(&state)
        .expect("the text assembly succeeds")
        .expect("a response comes back");
    assert_eq!(
        field(
            &field(
                &field(&assembled, "hidden_params"),
                "provider_specific_fields"
            ),
            "tool_results"
        ),
        json(r#"["t"]"#)
    );
}

#[test]
fn snapshot_and_metadata_merge_hidden_fields_into_existing_maps() {
    let fields = provider(r#"{"k":"v"}"#);
    let mut plain = chunk(r#"{"id":"c"}"#);
    merge_hidden_provider_fields(&mut plain, &fields).expect("the hidden map is created");
    assert_eq!(
        Json::Object(plain),
        json(r#"{"id":"c","hidden_params":{"provider_specific_fields":{"k":"v"}}}"#)
    );
    let mut null_hidden = chunk(r#"{"hidden_params":null}"#);
    merge_hidden_provider_fields(&mut null_hidden, &fields)
        .expect("the null hidden map is replaced");
    assert_eq!(
        Json::Object(null_hidden),
        json(r#"{"hidden_params":{"provider_specific_fields":{"k":"v"}}}"#)
    );
    let mut cost_hidden = chunk(r#"{"hidden_params":{"cost":1}}"#);
    merge_hidden_provider_fields(&mut cost_hidden, &fields).expect("existing hidden fields stay");
    assert_eq!(
        Json::Object(cost_hidden),
        json(r#"{"hidden_params":{"cost":1,"provider_specific_fields":{"k":"v"}}}"#)
    );
    let mut provider_hidden =
        chunk(r#"{"hidden_params":{"provider_specific_fields":{"k":"old","j":2}}}"#);
    merge_hidden_provider_fields(&mut provider_hidden, &fields)
        .expect("existing provider fields merge");
    assert_eq!(
        Json::Object(provider_hidden),
        json(r#"{"hidden_params":{"provider_specific_fields":{"k":"v","j":2}}}"#)
    );
}

#[test]
fn snapshot_and_metadata_reject_hidden_shapes_that_are_not_objects() {
    let fields = provider(r#"{"k":1}"#);
    let mut bad_hidden = chunk(r#"{"hidden_params":3}"#);
    assert!(matches!(
        merge_hidden_provider_fields(&mut bad_hidden, &fields),
        Err(StreamItemsError::HiddenParamsNotObject)
    ));
    let mut bad_provider = chunk(r#"{"hidden_params":{"provider_specific_fields":[1]}}"#);
    assert!(matches!(
        merge_hidden_provider_fields(&mut bad_provider, &fields),
        Err(StreamItemsError::ProviderFieldsNotObject)
    ));
    let mut untouched = chunk(r#"{"hidden_params":3}"#);
    let empty = provider("{}");
    merge_hidden_provider_fields(&mut untouched, &empty)
        .expect("empty provider fields never read the hidden map");
    assert_eq!(Json::Object(untouched), json(r#"{"hidden_params":3}"#));
}
