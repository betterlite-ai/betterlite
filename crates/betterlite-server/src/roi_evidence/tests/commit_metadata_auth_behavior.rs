#![cfg(test)]

use std::iter::repeat_with;

use super::commit_metadata;
use super::tests::{
    ENDLESS_GRAPHQL_PAGE, GraphQlRequestBody, NEXT_LINK, Script, Seen, github, github_with_token,
    graphql_page, json_reply, pull_detail, with_link,
};
use crate::roi_github::models::GRAPHQL_QUERY;

const REST_COMMITS: &str = r#"[
  {"sha":"c1","author":{"login":"octo"},
   "commit":{"message":"first","author":{"email":"dev@example.com"}}}
]"#;

fn request_body(seen: &Seen) -> GraphQlRequestBody {
    serde_json::from_str(seen.body.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn commit_metadata_without_a_token_reads_rest_commits() {
    let script = Script::new(vec![json_reply(200, REST_COMMITS)]);
    let github = github(script.clone());
    let metadata = commit_metadata(&github, "o/r", 7, &pull_detail(Some(4)))
        .await
        .unwrap();
    assert_eq!(metadata.count, 4);
    assert_eq!(metadata.commits.len(), 1);
    assert_eq!(metadata.commits[0].sha, "c1");
    assert_eq!(metadata.commits[0].message, "first");
    assert_eq!(metadata.commits[0].additions, None);
    assert_eq!(metadata.commits[0].changed_files, None);
    assert_eq!(metadata.authors[0].0, "octo");
    assert_eq!(metadata.authors[0].1, "dev@example.com");
    let seen = script.seen();
    assert_eq!(seen[0].method, "GET");
    assert_eq!(
        seen[0].url,
        "https://api.github.com/repos/o/r/pulls/7/commits"
    );
    assert_eq!(
        seen[0].query,
        vec![
            ("per_page".to_owned(), "100".to_owned()),
            ("page".to_owned(), "1".to_owned()),
        ]
    );
}

#[tokio::test]
async fn commit_metadata_rest_count_falls_back_to_the_collected_commits() {
    let commits = r#"[
      {"sha":"c1","commit":{"author":{"email":"a@example.com"}}},
      {"sha":"c2","commit":{"author":{"email":"b@example.com"}}}
    ]"#;
    let script = Script::new(vec![json_reply(200, commits)]);
    let github = github(script);
    let metadata = commit_metadata(&github, "o/r", 7, &pull_detail(None))
        .await
        .unwrap();
    assert_eq!(metadata.count, 2);
    assert_eq!(metadata.commits.len(), 2);
}

#[tokio::test]
async fn commit_metadata_rest_authors_default_missing_parts_to_empty_strings() {
    let commits = r#"[
      {"sha":"c1"},
      {"sha":"c2","author":{"login":""},"commit":{"author":{"email":""}}}
    ]"#;
    let script = Script::new(vec![json_reply(200, commits)]);
    let github = github(script);
    let metadata = commit_metadata(&github, "o/r", 7, &pull_detail(None))
        .await
        .unwrap();
    assert_eq!(metadata.authors[0], (String::new(), String::new()));
    assert_eq!(metadata.authors[1], (String::new(), String::new()));
}

#[tokio::test]
async fn commit_metadata_rest_stops_at_three_pages() {
    let script = Script::new(
        repeat_with(|| with_link(json_reply(200, "[]"), NEXT_LINK))
            .take(3)
            .collect(),
    );
    let github = github(script.clone());
    let error = commit_metadata(&github, "o/r", 7, &pull_detail(None))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub's pagination limit was reached. Narrow the date range."
    );
    assert_eq!(script.seen().len(), 3);
}

#[tokio::test]
async fn commit_metadata_with_a_token_posts_the_graphql_query() {
    let page = graphql_page("", false, 4, None);
    let script = Script::new(vec![json_reply(200, &page)]);
    let github = github_with_token(script.clone(), "secret", "https://api.github.com");
    let metadata = commit_metadata(&github, "o/r", 7, &pull_detail(Some(4)))
        .await
        .unwrap();
    assert_eq!(metadata.count, 4);
    assert!(metadata.commits.is_empty());
    let seen = script.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, "POST");
    assert_eq!(seen[0].url, "https://api.github.com/graphql");
    assert_eq!(seen[0].authorization.as_deref(), Some("Bearer secret"));
    assert_eq!(seen[0].content_type.as_deref(), Some("application/json"));
    let payload = request_body(&seen[0]);
    assert_eq!(payload.query, GRAPHQL_QUERY);
    assert_eq!(payload.variables.owner, "o");
    assert_eq!(payload.variables.name, "r");
    assert_eq!(payload.variables.number, 7);
    assert_eq!(payload.variables.cursor, None);
}

#[tokio::test]
async fn commit_metadata_graphql_stops_at_one_hundred_pages() {
    let script = Script::new(
        repeat_with(|| json_reply(200, ENDLESS_GRAPHQL_PAGE))
            .take(100)
            .collect(),
    );
    let github = github_with_token(script.clone(), "secret", "https://api.github.com");
    let error = commit_metadata(&github, "o/r", 7, &pull_detail(Some(0)))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub's pagination limit was reached. Narrow the date range."
    );
    assert_eq!(script.seen().len(), 100);
}

#[tokio::test]
async fn commit_metadata_graphql_uses_the_enterprise_api_path() {
    let page = graphql_page("", false, 0, None);
    let script = Script::new(vec![json_reply(200, &page)]);
    let github = github_with_token(script.clone(), "secret", "https://ghe.example.com/api/v3");
    commit_metadata(&github, "o/r", 7, &pull_detail(Some(0)))
        .await
        .unwrap();
    assert_eq!(script.seen()[0].url, "https://ghe.example.com/api/graphql");
}

#[tokio::test]
async fn commit_metadata_splits_the_repository_into_owner_and_name() {
    let page = graphql_page("", false, 0, None);
    let script = Script::new(vec![json_reply(200, &page)]);
    let github = github_with_token(script.clone(), "secret", "https://api.github.com");
    commit_metadata(&github, "solo", 7, &pull_detail(Some(0)))
        .await
        .unwrap();
    let payload = request_body(&script.seen()[0]);
    assert_eq!(payload.variables.owner, "solo");
    assert_eq!(payload.variables.name, "");
}
