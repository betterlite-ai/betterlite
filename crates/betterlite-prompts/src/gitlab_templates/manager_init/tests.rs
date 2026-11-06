#![cfg(test)]

use rstest::rstest;

use super::{ClientSource, scoped_prompts_path};
use crate::gitlab_client::GitLabClient;
use crate::gitlab_client::error::GitLabClientError;
use crate::gitlab_client::tests::{FakeTransport, answer, config_from};
use crate::gitlab_templates::error::ManagerError;
use crate::gitlab_templates::parse::FrontmatterParser;
use crate::gitlab_templates::tests::{SCOPED_CONFIG, build_manager};

#[rstest]
#[case::prompts_path(r#"{"prompts_path": "prompts"}"#, "prompts")]
#[case::folder(r#"{"folder": "prompts/chat"}"#, "prompts/chat")]
#[case::prompts_path_beats_folder(r#"{"prompts_path": "a", "folder": "b"}"#, "a")]
#[case::empty_prompts_path_defers_to_folder(r#"{"prompts_path": "", "folder": "b"}"#, "b")]
#[case::null_prompts_path_defers_to_folder(r#"{"prompts_path": null, "folder": "b"}"#, "b")]
#[case::slashes_are_trimmed(r#"{"prompts_path": "//a/b//"}"#, "a/b")]
#[case::only_slashes(r#"{"prompts_path": "/"}"#, "")]
#[case::nothing_configured("{}", "")]
fn the_prompts_path_comes_from_prompts_path_then_folder(
    #[case] config_json: &str,
    #[case] expected: &str,
) {
    assert_eq!(scoped_prompts_path(&config_from(config_json)), expected);
}

#[tokio::test]
async fn the_manager_stores_the_scope_and_loads_nothing_without_a_prompt_id() {
    let (built, transport) = build_manager(SCOPED_CONFIG, None, None, vec![]).await;
    assert_eq!(built.ok().unwrap().prompts_path, "prompts");
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn an_empty_prompt_id_loads_nothing() {
    let (built, transport) = build_manager(SCOPED_CONFIG, Some(""), None, vec![]).await;
    built.unwrap();
    assert!(transport.requests().is_empty());
}

#[tokio::test]
async fn a_prompt_id_is_loaded_while_the_manager_is_built() {
    let script = vec![answer!(200, "Hello")];
    let (built, transport) = build_manager(SCOPED_CONFIG, Some("chat/hi"), None, script).await;
    let manager = built.ok().unwrap();
    assert_eq!(manager.prompts.get("chat/hi").unwrap().content, "Hello");
    assert_eq!(transport.requests().len(), 1);
}

#[rstest]
#[case::given_ref(Some("release/1"), "release%2F1")]
#[case::empty_ref_is_ignored(Some(""), "main")]
#[case::no_ref(None, "main")]
#[tokio::test]
async fn a_ref_argument_sets_the_client_ref(
    #[case] ref_name: Option<&str>,
    #[case] expected: &str,
) {
    let script = vec![answer!(200, "x")];
    let (built, transport) = build_manager(SCOPED_CONFIG, Some("a"), ref_name, script).await;
    drop(built.ok().unwrap());
    let url = transport.requests()[0].url.clone();
    assert!(url.ends_with(&format!("?ref={expected}")), "{url}");
}

#[tokio::test]
async fn a_provided_client_is_used_instead_of_building_one_from_the_config() {
    let transport = FakeTransport::scripted(vec![answer!(200, "x")]);
    let client = GitLabClient::new(
        &config_from(r#"{"project": "own/project", "access_token": "t", "tag": "v9"}"#),
        transport.clone(),
    )
    .unwrap();
    let built = super::build(
        &config_from("{}"),
        Some("a"),
        None,
        ClientSource::Provided(client),
        FrontmatterParser::Yaml,
    )
    .await;
    drop(built.ok().unwrap());
    let url = transport.requests()[0].url.clone();
    assert!(url.contains("/projects/own%2Fproject/"), "{url}");
    assert!(url.ends_with("?ref=v9"), "{url}");
}

#[tokio::test]
async fn a_config_without_credentials_is_rejected_when_no_client_is_provided() {
    let (built, _) = build_manager(r#"{"project": "g/r"}"#, None, None, vec![]).await;
    assert!(matches!(
        built.err().unwrap(),
        ManagerError::Client(GitLabClientError::MissingCredentials)
    ));
}
