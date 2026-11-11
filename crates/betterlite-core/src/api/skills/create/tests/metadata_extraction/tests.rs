#![cfg(test)]

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::skills::create::metadata::requester_metadata;
use crate::api::skills::create_fixtures::{
    Fixture, call_with, hosted_params, logging, map, skill_creation,
};
use crate::api::skills::fixtures::{kwargs, object, text};

fn requester(value: &str) -> IndexMap<String, Json> {
    map(&[("requester", text(value))])
}

fn kwargs_metadata(value: &str) -> Json {
    object(&[("requester_metadata", Json::Object(requester(value)))])
}

fn extra_body_with_metadata(value: Json) -> IndexMap<String, Json> {
    let mut body = IndexMap::new();
    body.insert("metadata".to_owned(), value);
    body
}

#[test]
fn metadata_extraction_prefers_extra_body_metadata() {
    let mut values = kwargs();
    values.insert("metadata".to_owned(), kwargs_metadata("from kwargs"));
    let extra_body = extra_body_with_metadata(Json::Object(requester("from extra body")));
    assert_eq!(
        requester_metadata(&values, Some(&extra_body)),
        Some(&requester("from extra body"))
    );
}

#[test]
fn metadata_extraction_keeps_an_empty_extra_body_metadata() {
    let mut values = kwargs();
    values.insert("metadata".to_owned(), kwargs_metadata("from kwargs"));
    let extra_body = extra_body_with_metadata(object(&[]));
    assert_eq!(
        requester_metadata(&values, Some(&extra_body)),
        Some(&IndexMap::new())
    );
}

#[test]
fn requester_metadata_falls_back_to_kwargs_metadata() {
    let mut values = kwargs();
    values.insert("metadata".to_owned(), kwargs_metadata("from kwargs"));
    let mut extra_body = IndexMap::new();
    extra_body.insert("description".to_owned(), text("no metadata here"));
    assert_eq!(
        requester_metadata(&values, Some(&extra_body)),
        Some(&requester("from kwargs"))
    );
}

#[test]
fn metadata_extraction_ignores_a_non_dict_extra_body_metadata() {
    let mut values = kwargs();
    values.insert("metadata".to_owned(), kwargs_metadata("from kwargs"));
    let extra_body = extra_body_with_metadata(text("not a dict"));
    assert_eq!(
        requester_metadata(&values, Some(&extra_body)),
        Some(&requester("from kwargs"))
    );
}

#[test]
fn requester_metadata_ignores_a_non_dict_requester_metadata() {
    let mut values = kwargs();
    values.insert(
        "metadata".to_owned(),
        object(&[("requester_metadata", Json::List(Vec::new()))]),
    );
    assert_eq!(requester_metadata(&values, None), None);
}

#[test]
fn requester_metadata_ignores_a_non_dict_metadata() {
    let mut values = kwargs();
    values.insert("metadata".to_owned(), text("not a dict"));
    assert_eq!(requester_metadata(&values, None), None);
}

#[test]
fn requester_metadata_returns_none_without_metadata() {
    assert_eq!(requester_metadata(&kwargs(), None), None);
}

#[tokio::test]
async fn metadata_extraction_sends_the_extracted_metadata_to_the_hosted_handler() {
    let fixture = Fixture::new();
    let extra_body = extra_body_with_metadata(Json::Object(requester("from extra body")));
    let mut values = kwargs();
    values.insert("metadata".to_owned(), kwargs_metadata("from kwargs"));
    let hosted = call_with(hosted_params(), None, None, Some(&extra_body), &values);
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].metadata, Some(requester("from extra body")));
}
