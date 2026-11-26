use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;

use super::super::override_params::{RouterCredentials, override_vertex_params};
use super::support::{capture_logs, json_text, parameters};

fn credentials(litellm_params: Option<JsonObject>) -> RouterCredentials {
    RouterCredentials { litellm_params }
}

fn url_values() -> (Option<String>, Option<String>) {
    (
        Some("url-project".to_owned()),
        Some("us-central1".to_owned()),
    )
}

fn call(record: Option<&RouterCredentials>) -> (Option<String>, Option<String>) {
    let (project, location) = url_values();
    override_vertex_params(record, project, location)
}

#[test]
fn no_vector_store_record_returns_the_url_values() {
    let (logs, _guard) = capture_logs();
    assert_eq!(call(None), url_values());
    let text = logs.text();
    assert!(!text.contains("vector store"), "{text}");
}

#[test]
fn an_empty_litellm_params_warns_and_keeps_the_url_values() {
    let (logs, _guard) = capture_logs();
    let empty = credentials(Some(JsonObject::new()));
    assert_eq!(call(Some(&empty)), url_values());
    assert_eq!(call(Some(&credentials(None))), url_values());
    let text = logs.text();
    assert!(
        text.contains("Using vector store credentials to override vertex project and location")
    );
    assert!(text.contains("Vector store credentials found but litellm_params is empty"));
    assert_eq!(
        text.matches("Vector store credentials found but litellm_params is empty")
            .count(),
        2,
        "{text}"
    );
}

#[test]
fn non_empty_values_override_and_log_each_override() {
    let (logs, _guard) = capture_logs();
    let record = credentials(Some(parameters(&[
        ("vertex_project", json_text("store-project")),
        ("vertex_location", json_text("europe-west4")),
    ])));
    assert_eq!(
        call(Some(&record)),
        (
            Some("store-project".to_owned()),
            Some("europe-west4".to_owned())
        )
    );
    let text = logs.text();
    assert!(
        text.contains("Overriding vertex_project from URL"),
        "{text}"
    );
    assert!(text.contains("store-project"), "{text}");
    assert!(
        text.contains("Overriding vertex_location from URL"),
        "{text}"
    );
    assert!(text.contains("europe-west4"), "{text}");
    assert!(
        !text.contains("missing vertex_project in litellm_params"),
        "{text}"
    );
    assert!(
        !text.contains("missing vertex_location in litellm_params"),
        "{text}"
    );
}

#[test]
fn a_missing_project_warns_and_keeps_the_url_project() {
    let (logs, _guard) = capture_logs();
    let record = credentials(Some(parameters(&[(
        "vertex_location",
        json_text("europe-west4"),
    )])));
    let (project, location) = call(Some(&record));
    assert_eq!(project.as_deref(), Some("url-project"));
    assert_eq!(location.as_deref(), Some("europe-west4"));
    let text = logs.text();
    assert!(
        text.contains(
            "Vector store credentials found but missing vertex_project in litellm_params"
        ),
        "{text}"
    );
    assert!(
        !text.contains("missing vertex_location in litellm_params"),
        "{text}"
    );
}

#[test]
fn a_missing_location_warns_and_keeps_the_url_location() {
    let (logs, _guard) = capture_logs();
    let record = credentials(Some(parameters(&[(
        "vertex_project",
        json_text("store-project"),
    )])));
    let (project, location) = call(Some(&record));
    assert_eq!(project.as_deref(), Some("store-project"));
    assert_eq!(location.as_deref(), Some("us-central1"));
    let text = logs.text();
    assert!(
        text.contains(
            "Vector store credentials found but missing vertex_location in litellm_params"
        ),
        "{text}"
    );
    assert!(
        !text.contains("missing vertex_project in litellm_params"),
        "{text}"
    );
}

#[test]
fn an_empty_or_non_string_value_does_not_override() {
    let (logs, _guard) = capture_logs();
    let record = credentials(Some(parameters(&[
        ("vertex_project", json_text("")),
        ("vertex_location", Json::Bool(true)),
    ])));
    assert_eq!(call(Some(&record)), url_values());
    let text = logs.text();
    assert!(
        text.contains(
            "Vector store credentials found but missing vertex_project in litellm_params"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "Vector store credentials found but missing vertex_location in litellm_params"
        ),
        "{text}"
    );
}
