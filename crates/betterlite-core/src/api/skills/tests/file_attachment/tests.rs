#![cfg(test)]

use crate::api::skills::hook::response::{
    GeneratedFile, GeneratedFilesAttribute, HookResponse, ObjectResponse, attach_files_to_response,
};

fn generated_files() -> Vec<GeneratedFile> {
    vec![
        GeneratedFile {
            name: "chart.png".to_owned(),
            mime_type: "image/png".to_owned(),
            content_base64: "aGVsbG8=".to_owned(),
            size: 5,
        },
        GeneratedFile {
            name: "data.json".to_owned(),
            mime_type: "application/json".to_owned(),
            content_base64: "e30=".to_owned(),
            size: 2,
        },
    ]
}

const ATTACHED_FILES: &str = r#"[{"content_base64":"aGVsbG8=","mime_type":"image/png","name":"chart.png","size":5},{"content_base64":"e30=","mime_type":"application/json","name":"data.json","size":2}]"#;

fn dict_response() -> HookResponse {
    let fields = serde_json::from_str(r#"{"content": [], "stop_reason": "end_turn"}"#).unwrap();
    HookResponse::Dict(fields)
}

fn dict_response_with_files() -> HookResponse {
    let body = format!(
        r#"{{"content": [], "stop_reason": "end_turn", "_litellm_generated_files": {ATTACHED_FILES}}}"#
    );
    let fields = serde_json::from_str(&body).unwrap();
    HookResponse::Dict(fields)
}

fn object_response(attribute: GeneratedFilesAttribute, model_extra: Option<&str>) -> HookResponse {
    HookResponse::Object(ObjectResponse {
        content: Vec::new(),
        choices: Vec::new(),
        generated_files: attribute,
        model_extra: model_extra.map(|body| serde_json::from_str(body).unwrap()),
    })
}

fn object_response_with_files() -> HookResponse {
    let extra = format!(r#"{{"_litellm_generated_files": {ATTACHED_FILES}}}"#);
    object_response(
        GeneratedFilesAttribute::Writable(generated_files()),
        Some(&extra),
    )
}

#[test]
fn file_attachment_keeps_a_dict_response_unchanged_without_files() {
    let response = dict_response();
    assert_eq!(attach_files_to_response(response.clone(), &[]), response);
}

#[test]
fn file_attachment_keeps_an_object_response_unchanged_without_files() {
    let response = object_response(GeneratedFilesAttribute::Writable(Vec::new()), Some("{}"));
    assert_eq!(attach_files_to_response(response.clone(), &[]), response);
}

#[test]
fn file_attachment_sets_the_generated_files_key_on_a_dict_response() {
    assert_eq!(
        attach_files_to_response(dict_response(), &generated_files()),
        dict_response_with_files()
    );
}

#[test]
fn file_attachment_sets_the_attribute_on_a_writable_object_response() {
    let response = object_response(GeneratedFilesAttribute::Writable(Vec::new()), None);
    assert_eq!(
        attach_files_to_response(response, &generated_files()),
        object_response(GeneratedFilesAttribute::Writable(generated_files()), None)
    );
}

#[test]
fn file_attachment_swallows_a_rejecting_object_response() {
    let response = object_response(GeneratedFilesAttribute::Rejected, None);
    assert_eq!(
        attach_files_to_response(response.clone(), &generated_files()),
        response
    );
}

#[test]
fn file_attachment_writes_model_extra_when_the_object_has_it() {
    let body = format!(r#"{{"kept": true, "_litellm_generated_files": {ATTACHED_FILES}}}"#);
    assert_eq!(
        attach_files_to_response(
            object_response(GeneratedFilesAttribute::Rejected, Some(r#"{"kept": true}"#)),
            &generated_files()
        ),
        object_response(GeneratedFilesAttribute::Rejected, Some(&body))
    );
}

#[test]
fn file_attachment_writes_both_places_when_the_object_is_writable() {
    assert_eq!(
        attach_files_to_response(
            object_response(GeneratedFilesAttribute::Writable(Vec::new()), Some("{}")),
            &generated_files()
        ),
        object_response_with_files()
    );
}
