#![cfg(test)]

use std::iter::repeat_with;

use super::graphql_commits;
use super::tests::{
    ENDLESS_GRAPHQL_PAGE, GraphQlRequestBody, Script, Seen, github, graphql_error_script,
    graphql_page, json_reply, reply,
};

const NODE_A: &str = r#"{"commit":{"oid":"a","message":"one","additions":1,"deletions":2,"changedFilesIfAvailable":3,"author":{"email":"a@example.com","user":{"login":"octo"}}}}"#;
const NODE_B: &str =
    r#"{"commit":{"oid":"b","message":"two","additions":0,"deletions":0,"author":null}}"#;
const NODE_C: &str = r#"{"commit":{"oid":"c","message":"three","additions":5,"deletions":6,"changedFilesIfAvailable":null,"author":{"email":"c@example.com","user":null}}}"#;

const COMMIT_READ_ERROR: &str =
    "GitHub could not read commit metadata. Check repository permissions and API compatibility.";

fn request_body(seen: &Seen) -> GraphQlRequestBody {
    serde_json::from_str(seen.body.as_deref().unwrap()).unwrap()
}

#[tokio::test]
async fn graphql_commits_follow_the_end_cursor_and_accumulate() {
    let first = graphql_page(NODE_A, true, 3, Some("cursor-1"));
    let second = graphql_page(&format!("{NODE_B},{NODE_C}"), false, 3, None);
    let script = Script::new(vec![json_reply(200, &first), json_reply(200, &second)]);
    let github = github(script.clone());
    let metadata = graphql_commits(&github, "o/r", 7, None, 100).await.unwrap();
    assert_eq!(
        metadata
            .commits
            .iter()
            .map(|commit| commit.sha.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b", "c"]
    );
    assert_eq!(metadata.count, 3);
    assert_eq!(metadata.authors.len(), 3);
    assert_eq!(metadata.authors[0].0, "octo");
    assert_eq!(metadata.authors[0].1, "a@example.com");
    assert_eq!(metadata.authors[1], (String::new(), String::new()));
    assert_eq!(metadata.authors[2].1, "c@example.com");
    let seen = script.seen();
    assert_eq!(request_body(&seen[0]).variables.cursor, None);
    assert_eq!(
        request_body(&seen[1]).variables.cursor.as_deref(),
        Some("cursor-1")
    );
}

#[tokio::test]
async fn graphql_commits_maps_commit_file_counts() {
    let page = graphql_page(&format!("{NODE_A},{NODE_B},{NODE_C}"), false, 3, None);
    let script = Script::new(vec![json_reply(200, &page)]);
    let github = github(script);
    let metadata = graphql_commits(&github, "o/r", 7, None, 100).await.unwrap();
    assert_eq!(metadata.commits[0].additions, Some(1));
    assert_eq!(metadata.commits[0].deletions, Some(2));
    assert_eq!(metadata.commits[0].changed_files, Some(Some(3)));
    assert_eq!(metadata.commits[2].changed_files, Some(None));
}

#[tokio::test]
async fn graphql_commits_report_the_total_count_even_with_fewer_commits() {
    let page = graphql_page(NODE_A, false, 5, None);
    let script = Script::new(vec![json_reply(200, &page)]);
    let github = github(script);
    let metadata = graphql_commits(&github, "o/r", 7, None, 100).await.unwrap();
    assert_eq!(metadata.count, 5);
    assert_eq!(metadata.commits.len(), 1);
}

#[tokio::test]
async fn graphql_commits_refuse_to_start_with_no_pages_left() {
    let script = Script::new(Vec::new());
    let github = github(script.clone());
    let error = graphql_commits(&github, "o/r", 7, None, 0)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub's pagination limit was reached. Narrow the date range."
    );
    assert!(script.seen().is_empty());
}

#[tokio::test]
async fn graphql_commits_stop_at_the_page_limit() {
    let script = Script::new(
        repeat_with(|| json_reply(200, ENDLESS_GRAPHQL_PAGE))
            .take(100)
            .collect(),
    );
    let github = github(script.clone());
    let error = graphql_commits(&github, "o/r", 7, None, 100)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub's pagination limit was reached. Narrow the date range."
    );
    assert_eq!(script.seen().len(), 100);
}

#[tokio::test]
async fn graphql_commits_report_graphql_errors() {
    let github = github(graphql_error_script("permissions"));
    let error = graphql_commits(&github, "o/r", 7, None, 100)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), COMMIT_READ_ERROR);
}

#[tokio::test]
async fn graphql_commits_report_a_missing_repository() {
    let script = Script::new(vec![json_reply(200, r#"{"data":{"repository":null}}"#)]);
    let github = github(script);
    let error = graphql_commits(&github, "o/r", 7, None, 100)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), COMMIT_READ_ERROR);
}

#[tokio::test]
async fn graphql_commits_report_a_missing_pull_request() {
    let script = Script::new(vec![json_reply(
        200,
        r#"{"data":{"repository":{"pullRequest":null}}}"#,
    )]);
    let github = github(script);
    let error = graphql_commits(&github, "o/r", 7, None, 100)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), COMMIT_READ_ERROR);
}

#[tokio::test]
async fn graphql_commits_report_errors_next_to_data() {
    let page = graphql_page(NODE_A, false, 1, None);
    let body = page.replace(r#""errors":[]"#, r#""errors":[{"message":"denied"}]"#);
    let script = Script::new(vec![json_reply(200, &body)]);
    let github = github(script);
    let error = graphql_commits(&github, "o/r", 7, None, 100)
        .await
        .unwrap_err();
    assert_eq!(error.to_string(), COMMIT_READ_ERROR);
}

#[tokio::test]
async fn graphql_commits_report_unexpected_metadata() {
    let script = Script::new(vec![json_reply(200, "[]")]);
    let github = github(script);
    let error = graphql_commits(&github, "o/r", 7, None, 100)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub returned unexpected commit metadata."
    );
}

#[tokio::test]
async fn graphql_commits_report_http_failures() {
    let script = Script::new(vec![reply(401)]);
    let github = github(script);
    let error = graphql_commits(&github, "o/r", 7, None, 100)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Authentication failed. Check the configured GitHub token. (HTTP 401)"
    );
}

#[tokio::test]
async fn graphql_commits_do_not_retry_a_post() {
    let page = graphql_page(NODE_A, false, 1, None);
    let script = Script::new(vec![reply(429), json_reply(200, &page)]);
    let github = github(script.clone());
    let error = graphql_commits(&github, "o/r", 7, None, 100)
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub rate limit reached. Wait before syncing again. (HTTP 429)"
    );
    assert_eq!(script.seen().len(), 1);
}
