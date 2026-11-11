#![cfg(test)]

use crate::api::skills::create_fixtures::{
    Fixture, call_with, hosted_params, logging, map, skill_creation,
};
use crate::api::skills::fixtures::{kwargs, metadata_with, object, text};
use betterlite_models::decisions::Json;

fn skill_file(name: &str) -> Vec<Json> {
    vec![Json::Object(map(&[("filename", text(name))]))]
}

#[tokio::test]
async fn hosted_metadata_prefers_top_level_description_and_instructions() {
    let fixture = Fixture::new();
    let extra_body = map(&[
        ("description", text("from extra body")),
        ("instructions", text("from extra body")),
    ]);
    let mut values = kwargs();
    values.insert("description".to_owned(), text("from kwargs"));
    values.insert("instructions".to_owned(), text("from kwargs"));
    let hosted = call_with(hosted_params(), None, None, Some(&extra_body), &values);
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].description.as_deref(), Some("from kwargs"));
    assert_eq!(calls[0].instructions.as_deref(), Some("from kwargs"));
}

#[tokio::test]
async fn description_instructions_fall_back_to_extra_body() {
    let fixture = Fixture::new();
    let extra_body = map(&[
        ("description", text("from extra body")),
        ("instructions", text("from extra body")),
    ]);
    let values = kwargs();
    let hosted = call_with(hosted_params(), None, None, Some(&extra_body), &values);
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].description.as_deref(), Some("from extra body"));
    assert_eq!(calls[0].instructions.as_deref(), Some("from extra body"));
}

#[tokio::test]
async fn hosted_metadata_keeps_absent_description_and_instructions() {
    let fixture = Fixture::new();
    let values = kwargs();
    let hosted = call_with(hosted_params(), None, None, None, &values);
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].description, None);
    assert_eq!(calls[0].instructions, None);
}

#[tokio::test]
async fn display_title_files_use_the_original_arguments() {
    let fixture = Fixture::new();
    let given = skill_file("argument.md");
    let extra_body = map(&[
        ("display_title", text("Overlaid")),
        ("files", Json::List(skill_file("extra.md"))),
    ]);
    let values = kwargs();
    let hosted = call_with(
        hosted_params(),
        Some("Argument"),
        Some(&given),
        Some(&extra_body),
        &values,
    );
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].display_title.as_deref(), Some("Argument"));
    assert_eq!(calls[0].files, Some(given));
}

#[tokio::test]
async fn hosted_metadata_sends_user_id_and_principal() {
    let fixture = Fixture::new();
    let principal = object(&[("user_id", text("user-1"))]);
    let mut values = kwargs();
    values.insert("user_id".to_owned(), text("user-1"));
    values.insert("metadata".to_owned(), metadata_with(principal.clone()));
    let hosted = call_with(hosted_params(), None, None, None, &values);
    fixture
        .service()
        .create_skill(skill_creation(&hosted, &mut logging()))
        .await
        .expect("the hosted create succeeds");
    let calls = fixture.hosted.calls.lock().expect("the fake");
    assert_eq!(calls[0].user_id.as_deref(), Some("user-1"));
    assert_eq!(calls[0].principal, Some(principal));
}
