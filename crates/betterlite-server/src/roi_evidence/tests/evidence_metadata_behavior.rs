#![cfg(test)]

use std::iter::repeat_with;

use super::evidence;
use super::tests::{
    NEXT_LINK, Script, evidence_script, github, json_reply, pull_detail_json, pull_list_item,
    with_link,
};

const TWO_FILES: &str = r#"[
  {"filename":"a.rs","status":"modified","additions":3,"deletions":1},
  {"filename":"b.rs","status":"added","additions":7,"deletions":3}
]"#;

const ONE_COMMIT: &str = r#"[
  {"sha":"c1","author":{"login":"octo"},
   "commit":{"message":"first","author":{"email":"Dev@Example.com"}}}
]"#;

#[tokio::test]
async fn evidence_collects_every_field_of_a_pull() {
    let script = evidence_script(
        &pull_detail_json(7, Some("Octo"), Some(2), Some(1)),
        TWO_FILES,
        r#"{"email":"Octo@Example.com"}"#,
        ONE_COMMIT,
    );
    let github = github(script.clone());
    let evidence = evidence(&github, "o/r", &pull_list_item(7)).await.unwrap();
    assert_eq!(evidence.repo, "o/r");
    assert_eq!(evidence.number, 7);
    assert_eq!(evidence.title, "Pull 7");
    assert_eq!(evidence.body, "");
    assert_eq!(evidence.url, "https://github.com/o/r/pull/7");
    assert_eq!(evidence.login, "Octo");
    assert_eq!(evidence.profile_email, "octo@example.com");
    assert_eq!(
        evidence.commit_emails,
        Some(vec!["dev@example.com".to_owned()])
    );
    assert_eq!(
        evidence.emails,
        vec!["dev@example.com".to_owned(), "octo@example.com".to_owned()]
    );
    assert_eq!(evidence.merged_at, "2026-01-02T00:00:00Z");
    assert_eq!(evidence.head_sha, "sha-7");
    assert_eq!(evidence.additions, 10);
    assert_eq!(evidence.deletions, 4);
    assert_eq!(evidence.changed_files, 2);
    assert_eq!(evidence.files.len(), 2);
    assert_eq!(evidence.files[0].filename.as_deref(), Some("a.rs"));
    assert_eq!(evidence.commits.len(), 1);
    assert_eq!(evidence.commits[0].sha, "c1");
    assert_eq!(evidence.commit_count, 1);
    assert!(!evidence.incomplete_metadata);
}

#[tokio::test]
async fn evidence_requests_files_and_commits_for_the_listed_pull_number() {
    let script = evidence_script(
        &pull_detail_json(7, Some("Octo"), Some(2), Some(1)),
        TWO_FILES,
        r#"{"email":"Octo@Example.com"}"#,
        ONE_COMMIT,
    );
    let github = github(script.clone());
    evidence(&github, "o/r", &pull_list_item(7)).await.unwrap();
    let seen = script.seen();
    assert_eq!(seen.len(), 4);
    assert_eq!(seen[0].url, "https://api.github.com/repos/o/r/pulls/7");
    assert_eq!(
        seen[1].url,
        "https://api.github.com/repos/o/r/pulls/7/files"
    );
    assert_eq!(seen[2].url, "https://api.github.com/users/Octo");
    assert_eq!(
        seen[3].url,
        "https://api.github.com/repos/o/r/pulls/7/commits"
    );
}

#[tokio::test]
async fn evidence_keeps_the_detail_number_and_names_a_deleted_user() {
    let script = evidence_script(
        &pull_detail_json(99, None, Some(0), Some(0)),
        "[]",
        r#"{"email":""}"#,
        "[]",
    );
    let github = github(script.clone());
    let evidence = evidence(&github, "o/r", &pull_list_item(7)).await.unwrap();
    assert_eq!(evidence.number, 99);
    assert_eq!(evidence.login, "deleted-user");
    assert_eq!(evidence.profile_email, "");
    assert!(evidence.emails.is_empty());
    let seen = script.seen();
    assert_eq!(seen[0].url, "https://api.github.com/repos/o/r/pulls/7");
    assert_eq!(seen[2].url, "https://api.github.com/users/deleted-user");
}

#[tokio::test]
async fn evidence_matches_commit_authors_by_case_insensitive_login() {
    let commits = r#"[
      {"sha":"1","author":{"login":"octo"},"commit":{"author":{"email":"A@Example.com"}}},
      {"sha":"2","author":{"login":"OCTO"},"commit":{"author":{"email":"b@example.com"}}},
      {"sha":"3","author":{"login":"other"},"commit":{"author":{"email":"c@example.com"}}},
      {"sha":"4","author":null,"commit":{"author":{"email":"d@example.com"}}},
      {"sha":"5","author":{"login":""},"commit":{"author":{"email":"e@example.com"}}}
    ]"#;
    let script = evidence_script(
        &pull_detail_json(7, Some("Octo"), Some(0), Some(5)),
        "[]",
        r#"{"email":""}"#,
        commits,
    );
    let github = github(script);
    let evidence = evidence(&github, "o/r", &pull_list_item(7)).await.unwrap();
    assert_eq!(
        evidence.commit_emails,
        Some(vec!["a@example.com".to_owned(), "b@example.com".to_owned()])
    );
    assert_eq!(evidence.emails, vec!["a@example.com", "b@example.com"]);
}

#[tokio::test]
async fn evidence_keeps_an_unusable_commit_address_out_of_the_emails() {
    let commits = r#"[
      {"sha":"1","author":{"login":"octo"},"commit":{"message":"m","author":null}}
    ]"#;
    let script = evidence_script(
        &pull_detail_json(7, Some("Octo"), Some(0), Some(1)),
        "[]",
        r#"{"email":""}"#,
        commits,
    );
    let github = github(script);
    let evidence = evidence(&github, "o/r", &pull_list_item(7)).await.unwrap();
    assert_eq!(evidence.commit_emails, Some(vec![String::new()]));
    assert!(evidence.emails.is_empty());
}

#[tokio::test]
async fn evidence_flags_incomplete_metadata_when_the_counts_differ() {
    let script = evidence_script(
        &pull_detail_json(7, Some("Octo"), Some(5), Some(4)),
        TWO_FILES,
        r#"{"email":""}"#,
        ONE_COMMIT,
    );
    let github = github(script);
    let evidence = evidence(&github, "o/r", &pull_list_item(7)).await.unwrap();
    assert_eq!(evidence.changed_files, 5);
    assert_eq!(evidence.files.len(), 2);
    assert_eq!(evidence.commits.len(), 1);
    assert_eq!(evidence.commit_count, 4);
    assert!(evidence.incomplete_metadata);
}

#[tokio::test]
async fn evidence_falls_back_to_the_file_count_when_changed_files_is_absent() {
    let commits = r#"[
      {"sha":"1","author":null,"commit":{"author":null}},
      {"sha":"2","author":null,"commit":{"author":null}}
    ]"#;
    let script = evidence_script(
        &pull_detail_json(7, Some("Octo"), None, Some(2)),
        TWO_FILES,
        r#"{"email":""}"#,
        commits,
    );
    let github = github(script);
    let evidence = evidence(&github, "o/r", &pull_list_item(7)).await.unwrap();
    assert_eq!(evidence.changed_files, 2);
    assert_eq!(evidence.commits.len(), 2);
    assert!(!evidence.incomplete_metadata);
}

#[tokio::test]
async fn evidence_stops_file_paging_at_thirty_pages() {
    let mut replies = vec![json_reply(
        200,
        &pull_detail_json(7, Some("Octo"), Some(0), Some(0)),
    )];
    replies.extend(repeat_with(|| with_link(json_reply(200, "[]"), NEXT_LINK)).take(30));
    let script = Script::new(replies);
    let github = github(script.clone());
    let error = evidence(&github, "o/r", &pull_list_item(7))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub's pagination limit was reached. Narrow the date range."
    );
    assert_eq!(script.seen().len(), 31);
}

#[tokio::test]
async fn evidence_reports_unexpected_pull_details() {
    let script = Script::new(vec![json_reply(200, "{}")]);
    let github = github(script);
    let error = evidence(&github, "o/r", &pull_list_item(7))
        .await
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "GitHub returned unexpected pull request details."
    );
}
