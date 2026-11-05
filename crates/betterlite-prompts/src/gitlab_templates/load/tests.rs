#![cfg(test)]

use std::io;

use rstest::rstest;

use crate::gitlab_client::tests::answer;
use crate::gitlab_client::transport::GitLabResponse;
use crate::gitlab_templates::tests::{SCOPED_CONFIG, build_manager};

#[tokio::test]
async fn the_file_is_fetched_from_the_scoped_path_parsed_and_stored_under_the_prompt_id() {
    let file = "---\nmodel: gpt-4\ntemperature: 0.2\n---\nHello {{ name }}";
    let (built, transport) = build_manager(
        SCOPED_CONFIG,
        Some("gitlab::chat::greeting"),
        None,
        vec![answer!(200, file)],
    )
    .await;
    let manager = built.ok().unwrap();
    let template = manager.prompts.get("gitlab::chat::greeting").unwrap();
    assert_eq!(template.content, "Hello {{ name }}");
    assert_eq!(template.model.as_deref(), Some("gpt-4"));
    assert_eq!(template.template_id, "gitlab::chat::greeting");
    assert!(manager.prompts.get("chat/greeting").is_none());
    assert!(
        transport.requests()[0]
            .url
            .contains("/files/prompts%2Fchat%2Fgreeting.prompt/raw?")
    );
}

#[tokio::test]
async fn a_later_load_adds_to_the_prompts_and_uses_the_per_call_ref() {
    let script = vec![answer!(200, "one"), answer!(200, "two")];
    let (built, transport) = build_manager(SCOPED_CONFIG, Some("a"), None, script).await;
    let mut manager = built.ok().unwrap();
    manager
        .load_prompt_from_gitlab("b", Some("v2"))
        .await
        .unwrap();
    assert_eq!(manager.prompts.get("a").unwrap().content, "one");
    assert_eq!(manager.prompts.get("b").unwrap().content, "two");
    let urls: Vec<String> = transport
        .requests()
        .into_iter()
        .map(|request| request.url)
        .collect();
    assert!(urls[0].ends_with("/prompts%2Fa.prompt/raw?ref=main"));
    assert!(urls[1].ends_with("/prompts%2Fb.prompt/raw?ref=v2"));
}

#[tokio::test]
async fn loading_the_same_id_again_replaces_the_template() {
    let script = vec![answer!(200, "old"), answer!(200, "new")];
    let (built, _) = build_manager(SCOPED_CONFIG, Some("a"), None, script).await;
    let mut manager = built.ok().unwrap();
    manager.load_prompt_from_gitlab("a", None).await.unwrap();
    assert_eq!(manager.prompts.get("a").unwrap().content, "new");
}

#[rstest]
#[case::missing_file(vec![answer!(404, ""), answer!(404, "")])]
#[case::empty_file(vec![answer!(200, "")])]
#[tokio::test]
async fn a_missing_or_empty_file_stores_nothing_and_is_not_an_error(
    #[case] script: Vec<Result<GitLabResponse, io::Error>>,
) {
    let (built, _) = build_manager(SCOPED_CONFIG, Some("a"), None, script).await;
    assert!(built.ok().unwrap().prompts.get("a").is_none());
}

#[rstest]
#[case::unauthorized(
    answer!(401, ""),
    "Failed to load prompt 'gitlab::chat::hi' from GitLab: Authentication failed. Check your GitLab token and auth_method."
)]
#[case::transport_failure(
    Err(io::Error::other("connection reset")),
    "Failed to load prompt 'gitlab::chat::hi' from GitLab: Failed to fetch file 'prompts/chat/hi.prompt': the request failed: connection reset"
)]
#[tokio::test]
async fn a_fetch_failure_names_the_encoded_prompt_id_and_the_cause(
    #[case] response: Result<GitLabResponse, io::Error>,
    #[case] message: &str,
) {
    let (built, _) = build_manager(SCOPED_CONFIG, Some("chat/hi"), None, vec![response]).await;
    assert_eq!(built.err().unwrap().to_string(), message);
}

#[tokio::test]
async fn a_parse_failure_names_the_prompt_id_and_the_cause() {
    let (built, _) = build_manager(
        SCOPED_CONFIG,
        Some("chat/hi"),
        None,
        vec![answer!(200, "---\n- a\n---\nbody")],
    )
    .await;
    assert_eq!(
        built.err().unwrap().to_string(),
        "Failed to load prompt 'gitlab::chat::hi' from GitLab: frontmatter metadata must be a mapping"
    );
}
