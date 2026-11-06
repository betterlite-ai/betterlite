#![cfg(test)]

use rstest::rstest;

use super::models::{PAGINATION_ERROR, RepositoryItem};
use super::support::{Script, github, json_reply};

#[tokio::test]
async fn a_page_parses_into_the_adapter_models() {
    let script = Script::new(vec![json_reply(
        200,
        r#"[{"full_name":"octo/repo","visibility":"internal","private":true,"archived":true}]"#,
    )]);
    let github = github(script.clone());
    let (items, has_next) = github
        .fetch_page::<RepositoryItem>("user/repos", &[], 4, PAGINATION_ERROR)
        .await
        .unwrap();
    assert!(!has_next);
    assert_eq!(
        items,
        vec![RepositoryItem {
            full_name: "octo/repo".to_owned(),
            visibility: Some("internal".to_owned()),
            private: true,
            archived: true,
        }]
    );
}

#[rstest]
#[case::an_object_is_not_a_page(r#"{"full_name":"octo/repo"}"#, PAGINATION_ERROR)]
#[case::a_required_field_is_missing(r#"[{"visibility":"public"}]"#, PAGINATION_ERROR)]
#[case::an_empty_body_is_not_a_page("", PAGINATION_ERROR)]
#[case::null_is_not_a_page("null", PAGINATION_ERROR)]
#[case::the_caller_names_the_failure("{}", "GitHub returned an unexpected repository list.")]
#[tokio::test]
async fn an_invalid_page_reports_the_error_message(#[case] body: &str, #[case] expected: &str) {
    let script = Script::new(vec![json_reply(200, body)]);
    let github = github(script.clone());
    let error = github
        .fetch_page::<RepositoryItem>("user/repos", &[], 1, expected)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), expected);
}

#[tokio::test]
async fn the_page_request_asks_for_one_hundred_entries() {
    let script = Script::new(vec![json_reply(200, "[]")]);
    let github = github(script.clone());
    github
        .fetch_page::<RepositoryItem>(
            "user/repos",
            &[("state".to_owned(), "closed".to_owned())],
            7,
            PAGINATION_ERROR,
        )
        .await
        .unwrap();
    let seen = script.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(
        seen[0].query,
        vec![
            ("state".to_owned(), "closed".to_owned()),
            ("per_page".to_owned(), "100".to_owned()),
            ("page".to_owned(), "7".to_owned()),
        ]
    );
    assert_eq!(seen[0].url, "https://api.github.com/user/repos");
}
