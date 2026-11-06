#![cfg(test)]

use super::models::{GitHubHead, GitHubPullListItem, GitHubUser, PAGINATION_ERROR, RepositoryItem};
use super::pages;
use super::support::{Script, github, json_reply};

#[tokio::test]
async fn repository_fields_outside_the_model_are_ignored() {
    let script = Script::new(vec![json_reply(
        200,
        r#"[{"full_name":"octo/repo","visibility":null,"private":false,"archived":false,
             "owner":{"login":"octo"},"permissions":{"push":true},"fork":true}]"#,
    )]);
    let github = github(script.clone());
    let (items, _) =
        pages::fetch_page::<RepositoryItem, _>(&github, "user/repos", &[], 1, PAGINATION_ERROR)
            .await
            .unwrap();
    assert_eq!(
        items,
        vec![RepositoryItem {
            full_name: "octo/repo".to_owned(),
            visibility: None,
            private: false,
            archived: false,
        }]
    );
}

#[tokio::test]
async fn pull_request_fields_outside_the_model_are_ignored() {
    let script = Script::new(vec![json_reply(
        200,
        r#"[{"number":7,"title":"A change","updated_at":"2026-01-02T00:00:00Z",
             "merged_at":null,"html_url":"https://github.com/octo/repo/pull/7","body":null,
             "head":{"sha":"abc","ref":"topic","repo":{"id":2}},
             "user":{"login":"octo","id":99},"draft":true,"labels":[]}]"#,
    )]);
    let github = github(script.clone());
    let (items, _) = pages::fetch_page::<GitHubPullListItem, _>(
        &github,
        "repos/octo/repo/pulls",
        &[],
        1,
        PAGINATION_ERROR,
    )
    .await
    .unwrap();
    assert_eq!(
        items,
        vec![GitHubPullListItem {
            number: 7,
            html_url: "https://github.com/octo/repo/pull/7".to_owned(),
            merged_at: None,
            updated_at: "2026-01-02T00:00:00Z".to_owned(),
            title: "A change".to_owned(),
            body: None,
            head: Some(GitHubHead {
                sha: "abc".to_owned(),
            }),
            user: Some(GitHubUser {
                login: Some("octo".to_owned()),
            }),
        }]
    );
}
