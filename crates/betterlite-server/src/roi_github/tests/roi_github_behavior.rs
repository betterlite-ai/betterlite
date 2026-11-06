#![cfg(test)]

use rstest::rstest;

use super::models::{
    GRAPHQL_QUERY, GitHubHead, GitHubPullListItem, GraphQlAuthor, GraphQlCommit, GraphQlConnection,
    GraphQlError, GraphQlNode, GraphQlPageInfo, GraphQlResponse, PullDetail, RestAuthor,
    RestCommit, RestCommitContent,
};

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
        ]}}}},"errors":[]}"#;
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

#[test]
fn a_pull_item_without_an_html_url_or_actor_defaults_them() {
    let item: GitHubPullListItem =
        serde_json::from_str(r#"{"number":1,"updated_at":"2026-01-02T00:00:00Z","title":"t"}"#)
            .unwrap();
    assert_eq!(item.html_url, "");
    assert_eq!(item.merged_at, None);
    assert_eq!(item.body, None);
    assert_eq!(item.head, None);
    assert_eq!(item.user, None);
}

#[test]
fn a_missing_head_sha_defaults_to_empty() {
    let head: GitHubHead = serde_json::from_str("{}").unwrap();
    assert_eq!(head.sha, "");
}

#[test]
fn the_rest_models_default_their_text_fields() {
    let author: RestAuthor = serde_json::from_str("{}").unwrap();
    assert_eq!(author.email, "");
    let content: RestCommitContent = serde_json::from_str("{}").unwrap();
    assert_eq!(content.message, "");
    let commit: RestCommit = serde_json::from_str(r#"{"commit":{"author":{}}}"#).unwrap();
    assert_eq!(commit.commit.message, "");
    assert_eq!(commit.commit.author.unwrap().email, "");
}

#[test]
fn the_graphql_models_default_their_text_fields() {
    let author: GraphQlAuthor = serde_json::from_str("{}").unwrap();
    assert_eq!(author.email, "");
    let error: GraphQlError = serde_json::from_str("{}").unwrap();
    assert_eq!(error.message, "");
    let node: GraphQlNode = serde_json::from_str(
        r#"{"commit":{"oid":"a","message":"m","additions":1,"deletions":2,"author":{}}}"#,
    )
    .unwrap();
    assert_eq!(node.commit.author.unwrap().email, "");
}

#[derive(Clone, Copy)]
enum Record {
    PullListItem,
    PullDetail,
    GraphQlCommit,
    GraphQlPageInfo,
    GraphQlConnection,
}

fn parses(kind: Record, body: &str) -> bool {
    match kind {
        Record::PullListItem => serde_json::from_str::<GitHubPullListItem>(body).is_ok(),
        Record::PullDetail => serde_json::from_str::<PullDetail>(body).is_ok(),
        Record::GraphQlCommit => serde_json::from_str::<GraphQlCommit>(body).is_ok(),
        Record::GraphQlPageInfo => serde_json::from_str::<GraphQlPageInfo>(body).is_ok(),
        Record::GraphQlConnection => serde_json::from_str::<GraphQlConnection>(body).is_ok(),
    }
}

#[rstest]
#[case::pull_list_item_complete(
    Record::PullListItem,
    r#"{"number":1,"updated_at":"t","title":"x"}"#,
    true
)]
#[case::pull_list_item_needs_a_number(
    Record::PullListItem,
    r#"{"updated_at":"t","title":"x"}"#,
    false
)]
#[case::pull_list_item_needs_an_updated_at(
    Record::PullListItem,
    r#"{"number":1,"title":"x"}"#,
    false
)]
#[case::pull_list_item_needs_a_title(
    Record::PullListItem,
    r#"{"number":1,"updated_at":"t"}"#,
    false
)]
#[case::pull_detail_complete(
    Record::PullDetail,
    r#"{"number":1,"title":"t","html_url":"u","merged_at":"m","head":{}}"#,
    true
)]
#[case::pull_detail_needs_a_number(
    Record::PullDetail,
    r#"{"title":"t","html_url":"u","merged_at":"m","head":{}}"#,
    false
)]
#[case::pull_detail_needs_a_title(
    Record::PullDetail,
    r#"{"number":1,"html_url":"u","merged_at":"m","head":{}}"#,
    false
)]
#[case::pull_detail_needs_an_html_url(
    Record::PullDetail,
    r#"{"number":1,"title":"t","merged_at":"m","head":{}}"#,
    false
)]
#[case::pull_detail_needs_a_merged_at(
    Record::PullDetail,
    r#"{"number":1,"title":"t","html_url":"u","head":{}}"#,
    false
)]
#[case::pull_detail_needs_a_head(
    Record::PullDetail,
    r#"{"number":1,"title":"t","html_url":"u","merged_at":"m"}"#,
    false
)]
#[case::graphql_commit_complete(
    Record::GraphQlCommit,
    r#"{"oid":"a","message":"m","additions":1,"deletions":2}"#,
    true
)]
#[case::graphql_commit_needs_an_oid(
    Record::GraphQlCommit,
    r#"{"message":"m","additions":1,"deletions":2}"#,
    false
)]
#[case::graphql_commit_needs_a_message(
    Record::GraphQlCommit,
    r#"{"oid":"a","additions":1,"deletions":2}"#,
    false
)]
#[case::graphql_commit_needs_additions(
    Record::GraphQlCommit,
    r#"{"oid":"a","message":"m","deletions":2}"#,
    false
)]
#[case::graphql_commit_needs_deletions(
    Record::GraphQlCommit,
    r#"{"oid":"a","message":"m","additions":1}"#,
    false
)]
#[case::page_info_complete(Record::GraphQlPageInfo, r#"{"hasNextPage":false}"#, true)]
#[case::page_info_needs_has_next_page(Record::GraphQlPageInfo, r#"{"endCursor":null}"#, false)]
#[case::connection_complete(
    Record::GraphQlConnection,
    r#"{"totalCount":0,"pageInfo":{"hasNextPage":false},"nodes":[]}"#,
    true
)]
#[case::connection_needs_a_total_count(
    Record::GraphQlConnection,
    r#"{"pageInfo":{"hasNextPage":false},"nodes":[]}"#,
    false
)]
#[case::connection_needs_a_page_info(
    Record::GraphQlConnection,
    r#"{"totalCount":0,"nodes":[]}"#,
    false
)]
#[case::connection_needs_nodes(
    Record::GraphQlConnection,
    r#"{"totalCount":0,"pageInfo":{"hasNextPage":false}}"#,
    false
)]
fn a_required_field_must_be_present(
    #[case] kind: Record,
    #[case] body: &str,
    #[case] expected: bool,
) {
    assert_eq!(parses(kind, body), expected);
}
