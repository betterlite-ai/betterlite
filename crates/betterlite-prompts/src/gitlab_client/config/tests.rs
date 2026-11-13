#![cfg(test)]

use rstest::rstest;

use super::{DEFAULT_BASE_URL, ProjectId};
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::tests::{FakeTransport, config_from};

fn client(json_text: &str) -> GitLabClient<FakeTransport> {
    GitLabClient::new(&config_from(json_text), FakeTransport::default()).unwrap()
}

#[rstest]
#[case::branch_defaults_to_main(r#"{"project": "g/r", "access_token": "t"}"#, "main")]
#[case::explicit_branch(r#"{"project": "g/r", "access_token": "t", "branch": "dev"}"#, "dev")]
#[case::empty_branch_means_main(r#"{"project": "g/r", "access_token": "t", "branch": ""}"#, "main")]
#[case::null_branch_means_main(
    r#"{"project": "g/r", "access_token": "t", "branch": null}"#,
    "main"
)]
#[case::tag_beats_branch(
    r#"{"project": "g/r", "access_token": "t", "branch": "dev", "tag": "v1.2.3"}"#,
    "v1.2.3"
)]
#[case::tag_alone(r#"{"project": "g/r", "access_token": "t", "tag": "v2"}"#, "v2")]
#[case::empty_tag_falls_back_to_branch(
    r#"{"project": "g/r", "access_token": "t", "branch": "dev", "tag": ""}"#,
    "dev"
)]
#[case::null_tag_falls_back_to_branch(
    r#"{"project": "g/r", "access_token": "t", "branch": "dev", "tag": null}"#,
    "dev"
)]
fn ref_is_the_tag_else_the_branch_else_main(#[case] json_text: &str, #[case] expected: &str) {
    assert_eq!(client(json_text).ref_name, expected);
}

#[rstest]
#[case::default_base_url(
    r#"{"project": "g/r", "access_token": "t"}"#,
    "https://gitlab.com/api/v4"
)]
#[case::null_base_url(
    r#"{"project": "g/r", "access_token": "t", "base_url": null}"#,
    "https://gitlab.com/api/v4"
)]
#[case::custom_base_url(
    r#"{"project": "g/r", "access_token": "t", "base_url": "https://git.example/api/v4"}"#,
    "https://git.example/api/v4"
)]
fn base_url_defaults_to_gitlab_com(#[case] json_text: &str, #[case] expected: &str) {
    assert_eq!(client(json_text).base_url, expected);
}

#[test]
fn the_default_base_url_constant_is_gitlab_com() {
    assert_eq!(DEFAULT_BASE_URL, "https://gitlab.com/api/v4");
}

#[test]
fn a_numeric_project_is_read_as_a_number_and_a_path_as_text() {
    let numeric = config_from(r#"{"project": 42}"#);
    let path = config_from(r#"{"project": "group/repo"}"#);
    assert_eq!(numeric.project, Some(ProjectId::Number(42)));
    assert_eq!(path.project, Some(ProjectId::Path("group/repo".to_owned())));
}

#[test]
fn keys_the_client_does_not_read_are_ignored() {
    let config = config_from(
        r#"{"project": "g/r", "access_token": "t", "workspace": "w", "repository": "r"}"#,
    );
    assert_eq!(config.project, Some(ProjectId::Path("g/r".to_owned())));
}

#[test]
fn the_config_keeps_prompts_path_and_folder() {
    let config = config_from(r#"{"prompts_path": "p", "folder": "f"}"#);
    assert_eq!(config.prompts_path.as_deref(), Some("p"));
    assert_eq!(config.folder.as_deref(), Some("f"));
}

#[test]
fn the_config_debug_output_hides_the_token() {
    let config = config_from(r#"{"project": "g/r", "access_token": "glpat-secret-value"}"#);
    assert!(!format!("{config:?}").contains("glpat-secret-value"));
}
