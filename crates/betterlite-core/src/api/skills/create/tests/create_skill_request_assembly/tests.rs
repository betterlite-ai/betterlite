#![cfg(test)]

use betterlite_anthropic::skills::wire::CreateSkillRequest;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use crate::api::skills::create::error::CreateSkillError;
use crate::api::skills::create::request::assembled_request;
use crate::api::skills::create_fixtures::map;
use crate::api::skills::fixtures::text;

fn skill_file(name: &str) -> Vec<Json> {
    vec![Json::Object(map(&[("filename", text(name))]))]
}

#[test]
fn create_skill_request_builds_display_title_and_files() {
    let given = skill_file("SKILL.md");
    let request = assembled_request(Some("Title"), Some(&given), None)
        .expect("the arguments build a request");
    assert_eq!(
        request,
        CreateSkillRequest {
            display_title: Some("Title".to_owned()),
            files: Some(given),
            extra: IndexMap::new(),
        }
    );
}

#[test]
fn create_skill_request_omits_absent_display_title_and_files() {
    let request = assembled_request(None, None, None).expect("an empty request builds");
    assert_eq!(
        request,
        CreateSkillRequest {
            display_title: None,
            files: None,
            extra: IndexMap::new(),
        }
    );
}

#[test]
fn create_skill_request_overlays_extra_body_keys() {
    let mut extra_body = IndexMap::new();
    extra_body.insert("description".to_owned(), text("what it does"));
    extra_body.insert("metadata".to_owned(), Json::Object(map(&[])));
    let request =
        assembled_request(None, None, Some(&extra_body)).expect("the extra body builds a request");
    assert_eq!(request.extra, extra_body);
}

#[test]
fn create_skill_request_extra_body_overrides_the_arguments() {
    let given = skill_file("argument.md");
    let mut extra_body = IndexMap::new();
    extra_body.insert("display_title".to_owned(), text("Overlaid"));
    extra_body.insert("files".to_owned(), Json::List(skill_file("extra.md")));
    let request = assembled_request(Some("Argument"), Some(&given), Some(&extra_body))
        .expect("the extra body overrides the arguments");
    assert_eq!(request.display_title.as_deref(), Some("Overlaid"));
    assert_eq!(request.files, Some(skill_file("extra.md")));
}

#[test]
fn create_skill_request_extra_body_null_clears_the_argument() {
    let given = skill_file("SKILL.md");
    let mut extra_body = IndexMap::new();
    extra_body.insert("display_title".to_owned(), Json::Null);
    extra_body.insert("files".to_owned(), Json::Null);
    let request = assembled_request(Some("Argument"), Some(&given), Some(&extra_body))
        .expect("the null overlay clears the arguments");
    assert_eq!(
        request,
        CreateSkillRequest {
            display_title: None,
            files: None,
            extra: IndexMap::new(),
        }
    );
}

#[test]
fn create_skill_request_rejects_a_wrong_typed_display_title() {
    let mut extra_body = IndexMap::new();
    extra_body.insert("display_title".to_owned(), Json::Bool(true));
    let error = assembled_request(None, None, Some(&extra_body))
        .expect_err("a boolean title is not declared");
    assert!(matches!(
        error,
        CreateSkillError::FieldType {
            name: "display_title"
        }
    ));
    assert_eq!(
        error.to_string(),
        "the display_title field does not have the declared type"
    );
}

#[test]
fn create_skill_request_rejects_wrong_typed_files() {
    let mut extra_body = IndexMap::new();
    extra_body.insert("files".to_owned(), text("SKILL.md"));
    let error =
        assembled_request(None, None, Some(&extra_body)).expect_err("a string is not a file list");
    assert!(matches!(
        error,
        CreateSkillError::FieldType { name: "files" }
    ));
}
