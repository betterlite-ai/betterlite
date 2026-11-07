#![cfg(test)]

use betterlite_roi::records::{ROIPullCommit, ROIPullFile};

use super::adapters::{
    RepositoryValue, graphql_commit_evidence, pull_file_evidence, repository_values,
    rest_commit_evidence,
};
use super::models::{
    GraphQlCommit, GraphQlNode, PullFile, RepositoryItem, RestCommit, RestCommitContent,
};

#[test]
fn repository_values_take_the_normalized_visibility_and_archive_flag() {
    let repositories = vec![
        RepositoryItem {
            full_name: "octo/one".to_owned(),
            visibility: Some("internal".to_owned()),
            private: false,
            archived: true,
        },
        RepositoryItem {
            full_name: "octo/two".to_owned(),
            visibility: None,
            private: true,
            archived: false,
        },
        RepositoryItem {
            full_name: "octo/three".to_owned(),
            visibility: None,
            private: false,
            archived: false,
        },
    ];
    assert_eq!(
        repository_values(&repositories),
        vec![
            RepositoryValue {
                full_name: "octo/one".to_owned(),
                visibility: "internal".to_owned(),
                archived: true,
            },
            RepositoryValue {
                full_name: "octo/two".to_owned(),
                visibility: "private".to_owned(),
                archived: false,
            },
            RepositoryValue {
                full_name: "octo/three".to_owned(),
                visibility: "public".to_owned(),
                archived: false,
            },
        ]
    );
}

#[test]
fn pull_file_evidence_keeps_every_field_including_nulls() {
    let file = PullFile {
        filename: Some("src/lib.rs".to_owned()),
        status: Some("modified".to_owned()),
        additions: Some(4),
        deletions: None,
    };
    assert_eq!(
        pull_file_evidence(&file),
        ROIPullFile {
            filename: Some("src/lib.rs".to_owned()),
            status: Some("modified".to_owned()),
            additions: Some(4),
            deletions: None,
        }
    );
}

#[test]
fn rest_commit_evidence_carries_sha_and_message_only() {
    let commit = RestCommit {
        sha: "abc123".to_owned(),
        author: None,
        commit: RestCommitContent {
            message: "Fix the thing".to_owned(),
            author: None,
        },
    };
    assert_eq!(
        rest_commit_evidence(&commit),
        ROIPullCommit {
            sha: "abc123".to_owned(),
            message: "Fix the thing".to_owned(),
            additions: None,
            deletions: None,
            changed_files: None,
        }
    );
}

#[test]
fn graphql_commit_evidence_carries_the_file_counts() {
    let node = GraphQlNode {
        commit: GraphQlCommit {
            oid: "def456".to_owned(),
            message: "Add the thing".to_owned(),
            additions: 12,
            deletions: 3,
            changed_files_if_available: Some(2),
            author: None,
        },
    };
    assert_eq!(
        graphql_commit_evidence(&node),
        ROIPullCommit {
            sha: "def456".to_owned(),
            message: "Add the thing".to_owned(),
            additions: Some(12),
            deletions: Some(3),
            changed_files: Some(Some(2)),
        }
    );
}

#[test]
fn graphql_commit_evidence_keeps_an_absent_changed_file_count_present_and_null() {
    let node = GraphQlNode {
        commit: GraphQlCommit {
            oid: "def456".to_owned(),
            message: "Add the thing".to_owned(),
            additions: 12,
            deletions: 3,
            changed_files_if_available: None,
            author: None,
        },
    };
    assert_eq!(
        graphql_commit_evidence(&node).changed_files,
        Some(None),
        "LiteLLM writes changed_files: None rather than leaving the key out"
    );
}
