use axum::http::StatusCode;
use betterlite_db::auxiliary_repository::repository::TableError;

use super::support::{FakeRows, FakeTeams, access, caller, row, store_failure};
use crate::memory_records::access::assert_write_access;
use crate::memory_records::error::duplicate_key_error;
use crate::memory_records::store::find_visible_memory_or_none;
use crate::memory_records::unique::{is_table_unique_violation, is_unique_violation};

#[test]
fn typed_p2002_code() {
    assert!(is_unique_violation(Some("P2002"), "no keyword here"));
    assert!(!is_unique_violation(Some("P2025"), "no keyword here"));
    assert!(!is_unique_violation(None, "no keyword here"));
    assert!(!is_table_unique_violation(&store_failure()));
}

#[test]
fn substring_detection() {
    assert!(is_unique_violation(
        None,
        "PrismaClientKnownRequestError P2002"
    ));
    assert!(is_unique_violation(
        None,
        "Unique constraint failed on the fields: (`key`)"
    ));
    assert!(is_unique_violation(None, "a unique constraint failed"));
    assert!(is_unique_violation(
        None,
        "UniqueViolation: duplicate key value"
    ));
    assert!(!is_unique_violation(None, "UNIQUE VIOLATION"));
    assert!(!is_unique_violation(
        None,
        "relation \"LiteLLM_MemoryTable\" does not exist"
    ));
}

#[test]
fn global_key_uniqueness() {
    let violation = TableError::Publish {
        message: "duplicate key value violates unique constraint \"LiteLLM_MemoryTable_key_key\""
            .to_owned(),
    };
    assert!(is_table_unique_violation(&violation));
    let conflict = duplicate_key_error("team/notes");
    assert_eq!(conflict.status, StatusCode::CONFLICT);
    assert_eq!(
        conflict.detail,
        "Memory with key 'team/notes' already exists."
    );
}

#[test]
fn create_duplicate_409() {
    let conflict = duplicate_key_error("note");
    assert_eq!(conflict.status, StatusCode::CONFLICT);
    assert_eq!(conflict.detail, "Memory with key 'note' already exists.");
}

#[test]
fn upsert_race_detection() {
    let violation = TableError::Publish {
        message: "UniqueViolation on the key index".to_owned(),
    };
    assert!(is_table_unique_violation(&violation));
    assert!(!is_table_unique_violation(&TableError::MissingRow));
    assert!(!is_table_unique_violation(&store_failure()));
}

#[tokio::test]
async fn invisible_duplicate_409() {
    let rows = FakeRows::new(None);
    let bob = caller(Some("bob"), Some("team-a"), None);
    let visible = find_visible_memory_or_none(&rows, "note", &bob)
        .await
        .unwrap();
    assert_eq!(visible, None);
    assert_eq!(duplicate_key_error("note").status, StatusCode::CONFLICT);
}

#[tokio::test]
async fn visible_duplicate_recheck_write_access() {
    let rows = FakeRows::new(Some(row("mem-1", "note", Some("alice"), None)));
    let owner = caller(Some("alice"), None, None);
    let visible = find_visible_memory_or_none(&rows, "note", &owner)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(visible.memory_id, "mem-1");

    let teams = FakeTeams::failing();
    assert_write_access(&owner, &visible, &teams, &access(Vec::new(), false))
        .await
        .unwrap();
    let teammate = caller(Some("bob"), Some("team-a"), None);
    let error = assert_write_access(&teammate, &visible, &teams, &access(Vec::new(), false))
        .await
        .unwrap_err();
    assert_eq!(error.status, StatusCode::FORBIDDEN);
}
