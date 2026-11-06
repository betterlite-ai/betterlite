#![cfg(test)]

use super::models::{GRAPHQL_QUERY, GraphQlResponse, PullDetail, RestCommit, RestCommitContent};

#[test]
fn the_graphql_query_matches_litellms_declaration() {
    assert_eq!(
        GRAPHQL_QUERY,
        "query($owner:String!, $name:String!, $number:Int!, $cursor:String) {\n  repository(owner:$owner, name:$name) { pullRequest(number:$number) {\n    commits(first:100, after:$cursor) {\n      totalCount pageInfo { hasNextPage endCursor }\n      nodes { commit { oid message additions deletions changedFilesIfAvailable\n        author { email user { login } } } }\n    }\n  } }\n}"
    );
}

#[test]
fn a_graphql_response_parses_its_nested_connection() {
    let body = r#"{"data":{"repository":{"pullRequest":{"commits":{
        "totalCount":2,
        "pageInfo":{"hasNextPage":true,"endCursor":"cursor-1"},
        "nodes":[
          {"commit":{"oid":"a1","message":"one","additions":1,"deletions":2,
            "changedFilesIfAvailable":3,"author":{"email":"dev@example.com","user":{"login":"octo"}}}},
          {"commit":{"oid":"a2","message":"two","additions":0,"deletions":0,
            "changedFilesIfAvailable":null,"author":null}}
        ]}}}}},"errors":[]}"#;
    let parsed: GraphQlResponse = serde_json::from_str(body).unwrap();
    let pull_request = parsed
        .data
        .unwrap()
        .repository
        .unwrap()
        .pull_request
        .unwrap();
    assert_eq!(pull_request.commits.total_count, 2);
    assert!(pull_request.commits.page_info.has_next_page);
    assert_eq!(
        pull_request.commits.page_info.end_cursor.as_deref(),
        Some("cursor-1")
    );
    let first = &pull_request.commits.nodes[0].commit;
    assert_eq!(first.oid, "a1");
    assert_eq!(first.changed_files_if_available, Some(3));
    assert_eq!(first.author.as_ref().unwrap().email, "dev@example.com",);
    let second = &pull_request.commits.nodes[1].commit;
    assert_eq!(second.changed_files_if_available, None);
    assert!(second.author.is_none());
    assert!(parsed.errors.is_empty());
}

#[test]
fn graphql_errors_and_a_missing_repository_are_representable() {
    let body = r#"{"data":{"repository":null},"errors":[{"message":"permissions"}]}"#;
    let parsed: GraphQlResponse = serde_json::from_str(body).unwrap();
    assert!(parsed.data.unwrap().repository.is_none());
    assert_eq!(parsed.errors[0].message, "permissions");
    let empty: GraphQlResponse = serde_json::from_str("{}").unwrap();
    assert!(empty.data.is_none());
    assert!(empty.errors.is_empty());
}

#[test]
fn a_rest_commit_defaults_its_content_and_sha() {
    let commit: RestCommit = serde_json::from_str("{}").unwrap();
    assert_eq!(commit.sha, "");
    assert_eq!(commit.commit, RestCommitContent::default());
    assert!(commit.author.is_none());
}

#[test]
fn pull_details_default_the_counts_and_keep_absent_totals() {
    let body = r#"{"number":5,"title":"T","html_url":"https://github.com/o/r/pull/5",
        "merged_at":"2026-02-03T04:05:06Z","head":{"sha":"abc"}}"#;
    let detail: PullDetail = serde_json::from_str(body).unwrap();
    assert_eq!(detail.number, 5);
    assert_eq!(detail.additions, 0);
    assert_eq!(detail.deletions, 0);
    assert_eq!(detail.changed_files, None);
    assert_eq!(detail.commits, None);
    assert!(detail.body.is_none());
    assert!(detail.user.is_none());
    assert_eq!(detail.head.sha, "abc");
}
