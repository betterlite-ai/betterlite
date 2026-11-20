use std::str::from_utf8;

use betterlite_http::proxy::multipart::MultipartPart;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support::{params, request_file};
use crate::api::videos_http_transport::error::SendFailure;
use crate::api::videos_http_transport::multipart::{
    MAX_RECURSE_DEPTH, boundary, flattened, with_files,
};
use crate::api::videos_http_transport::video_config::Params;

fn names(parts: &[MultipartPart]) -> Vec<&str> {
    parts.iter().map(|part| part.name.as_str()).collect()
}

fn texts(parts: &[MultipartPart]) -> Vec<(&str, &str)> {
    parts
        .iter()
        .map(|part| {
            (
                part.name.as_str(),
                from_utf8(&part.data).expect("the part is text"),
            )
        })
        .collect()
}

fn chain_params(objects: usize) -> Params {
    let mut value = Json::Text("x".to_owned());
    for _ in 0..objects {
        value = Json::Object(IndexMap::from([("k".to_owned(), value)]));
    }
    Params::from([("a".to_owned(), value)])
}

#[test]
fn data_fields_come_before_file_parts() {
    let data = params(r#"{"prompt":"p","seconds":"4"}"#);
    let files = [
        request_file("first", Some("a.mp4"), Some("video/mp4")),
        request_file("second", None, None),
    ];
    let parts = with_files(&data, &files).expect("the parts");
    assert_eq!(names(&parts), ["prompt", "seconds", "first", "second"]);
}

#[test]
fn a_file_name_is_kept_and_an_empty_one_is_dropped() {
    let files = [
        request_file("a", Some(""), Some("image/png")),
        request_file("b", Some("b.png"), None),
    ];
    let parts = with_files(&Params::new(), &files).expect("the parts");
    assert_eq!(parts[0].file_name, None);
    assert_eq!(parts[0].content_type.as_deref(), Some("image/png"));
    assert_eq!(parts[1].file_name.as_deref(), Some("b.png"));
    assert_eq!(parts[1].content_type.as_deref(), Some("image/png"));
}

#[test]
fn a_missing_file_type_is_guessed_from_the_name() {
    let files = [
        request_file("a", Some("clip.mp4"), None),
        request_file("b", Some("blob.unknown-extension"), None),
        request_file("c", None, None),
    ];
    let parts = with_files(&Params::new(), &files).expect("the parts");
    assert_eq!(parts[0].content_type.as_deref(), Some("video/mp4"));
    assert_eq!(
        parts[1].content_type.as_deref(),
        Some("application/octet-stream")
    );
    assert_eq!(parts[2].content_type, None);
}

#[test]
fn a_nested_object_flattens_into_bracketed_names() {
    let parts =
        flattened(&params(r#"{"model":"m","metadata":{"a":1,"b":"x"}}"#)).expect("the fields");
    assert_eq!(
        texts(&parts),
        [("model", "m"), ("metadata[a]", "1"), ("metadata[b]", "x"),]
    );
}

#[test]
fn a_list_flattens_into_repeated_names_in_order() {
    let parts = flattened(&params(r#"{"tags":["x","y"],"after":"z"}"#)).expect("the fields");
    assert_eq!(
        texts(&parts),
        [("tags[]", "x"), ("tags[]", "y"), ("after", "z")]
    );
}

#[test]
fn a_null_and_an_empty_string_are_dropped() {
    let parts = flattened(&params(r#"{"a":null,"b":"","c":"x"}"#)).expect("the fields");
    assert_eq!(texts(&parts), [("c", "x")]);
}

#[test]
fn a_boolean_and_a_number_keep_their_json_text() {
    let parts = flattened(&params(r#"{"b":true,"n":1.5,"i":2}"#)).expect("the fields");
    assert_eq!(texts(&parts), [("b", "true"), ("n", "1.5"), ("i", "2")]);
}

#[test]
fn nesting_at_the_limit_is_accepted_and_one_past_it_is_refused() {
    let accepted = flattened(&chain_params(MAX_RECURSE_DEPTH)).expect("the depth at the limit");
    assert_eq!(accepted.len(), 1);
    let refused = flattened(&chain_params(MAX_RECURSE_DEPTH.saturating_add(1)))
        .expect_err("one past the limit");
    let SendFailure::Other { message } = refused else {
        panic!("the depth failure is not a status");
    };
    assert_eq!(message, "form field nesting exceeds max depth");
}

#[test]
fn a_nested_object_in_the_files_path_is_refused() {
    let refused = with_files(&params(r#"{"a":{"b":1}}"#), &[]).expect_err("the nested value");
    let SendFailure::Other { message } = refused else {
        panic!("the value failure is not a status");
    };
    assert_eq!(message, "the a form field is not a primitive value");
}

#[test]
fn a_list_of_primitives_repeats_in_the_files_path() {
    let parts = with_files(&params(r#"{"a":[1,true,"x"]}"#), &[]).expect("the parts");
    assert_eq!(texts(&parts), [("a", "1"), ("a", "true"), ("a", "x")]);
}

#[test]
fn a_list_inside_another_keeps_one_pair_of_brackets_per_level() {
    let nested = flattened(&params(r#"{"a":[[1]]}"#)).expect("the fields");
    assert_eq!(texts(&nested), [("a[][]", "1")]);
    let objects = flattened(&params(r#"{"a":[{"b":1}]}"#)).expect("the fields");
    assert_eq!(texts(&objects), [("a[][b]", "1")]);
}

#[test]
fn the_boundary_is_stable_for_the_same_parts_and_follows_the_file_facts() {
    let parts = flattened(&params(r#"{"a":"x"}"#)).expect("the fields");
    assert_eq!(boundary(&parts), boundary(&parts));
    assert_eq!(boundary(&parts).len(), 64);
    let other = flattened(&params(r#"{"a":"y"}"#)).expect("the fields");
    assert_ne!(boundary(&parts), boundary(&other));
    let mp4 =
        with_files(&Params::new(), &[request_file("f", Some("a.mp4"), None)]).expect("the parts");
    let png =
        with_files(&Params::new(), &[request_file("f", Some("a.png"), None)]).expect("the parts");
    assert_ne!(boundary(&mp4), boundary(&png));
}
