#![cfg(test)]

use rstest::rstest;

use super::{
    VertexLocationError, speech_to_text_host, validate_vertex_location, validate_vertex_project,
};

#[rstest]
#[case::global("global")]
#[case::multi_region("us")]
#[case::region("us-central1")]
#[case::single_letter("a")]
#[case::digits_and_hyphens("z9-")]
fn lowercase_locations_are_accepted(#[case] location: &str) {
    assert_eq!(validate_vertex_location(Some(location)), Ok(location));
}

#[rstest]
#[case::empty("")]
#[case::uppercase("US")]
#[case::uppercase_rest("uS")]
#[case::digit_first("1us")]
#[case::hyphen_first("-us")]
#[case::underscore("us_central1")]
#[case::host_injection("attacker.example/")]
#[case::fragment("evil.com#")]
#[case::trailing_newline("us\n")]
#[case::trailing_space("us-central1 ")]
fn locations_that_could_move_the_host_are_rejected(#[case] location: &str) {
    assert_eq!(
        validate_vertex_location(Some(location)),
        Err(VertexLocationError::Invalid)
    );
}

#[test]
fn a_missing_location_is_required() {
    let error = validate_vertex_location(None).unwrap_err();
    assert_eq!(error, VertexLocationError::Required);
    assert_eq!(error.to_string(), "vertex_location is required");
    assert_eq!(
        VertexLocationError::Invalid.to_string(),
        "invalid vertex_location format"
    );
}

#[rstest]
#[case::empty("")]
#[case::parent("a..b")]
#[case::slash("bad/project")]
#[case::question("bad?project")]
#[case::hash("bad#project")]
#[case::backslash("bad\\project")]
#[case::colon("bad:project")]
#[case::space("bad project")]
#[case::tab("bad\tproject")]
#[case::newline("bad\nproject")]
#[case::carriage_return("bad\rproject")]
fn projects_that_could_move_the_request_are_rejected(#[case] project: &str) {
    let error = validate_vertex_project(project).unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("the vertex_project '{project}' cannot be used in a URL")
    );
    assert_eq!(
        error,
        VertexLocationError::Project {
            project: project.to_owned()
        }
    );
}

#[rstest]
#[case::plain("my-project")]
#[case::single_dot("my.project")]
#[case::digits("project-123")]
fn safe_projects_are_accepted(#[case] project: &str) {
    assert_eq!(validate_vertex_project(project), Ok(project));
}

#[rstest]
#[case::global("global", "speech.googleapis.com")]
#[case::multi_region("us", "us-speech.googleapis.com")]
#[case::region("europe-west4", "europe-west4-speech.googleapis.com")]
fn speech_hosts_follow_the_location(#[case] location: &str, #[case] expected: &str) {
    assert_eq!(speech_to_text_host(location), expected);
}
