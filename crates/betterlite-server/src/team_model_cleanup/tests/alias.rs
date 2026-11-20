use async_trait::async_trait;

use crate::team_model_cleanup::alias::{AliasMap, AliasRow, AliasRows, delete_team_model_alias};
use crate::team_model_cleanup::error::CleanupError;

use super::support::{FakeStore, alias_row, aliases};

#[tokio::test]
async fn alias_delete() {
    let rows = vec![
        alias_row(
            1,
            Some("t1"),
            Some(r#"{"first":"old","second":"old","keep":"other"}"#),
        ),
        alias_row(2, None, Some(r#"{"orphan":"old"}"#)),
        alias_row(3, Some("t2"), Some(r#"{"current":"new"}"#)),
        alias_row(4, Some("t1"), None),
    ];
    let store = FakeStore::new("t1", Some(Vec::new()), rows, Vec::new());
    let removed = delete_team_model_alias("old", &store).await.unwrap();
    assert_eq!(removed, vec![("t1".to_owned(), "first".to_owned())]);
    let saved: Vec<i64> = store
        .saved_aliases
        .lock()
        .unwrap()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(saved, vec![1_i64, 2_i64]);
    assert_eq!(
        store.remaining_aliases(),
        vec![
            Some(aliases(r#"{"second":"old","keep":"other"}"#)),
            Some(aliases("{}")),
            Some(aliases(r#"{"current":"new"}"#)),
            None,
        ]
    );
}

#[tokio::test]
async fn a_name_no_alias_maps_to_updates_nothing() {
    let rows = vec![alias_row(1, Some("t1"), Some(r#"{"keep":"other"}"#))];
    let store = FakeStore::new("t1", Some(Vec::new()), rows, Vec::new());
    let removed = delete_team_model_alias("old", &store).await.unwrap();
    assert!(removed.is_empty());
    assert!(store.saved_aliases.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_non_text_alias_value_never_matches() {
    let rows = vec![alias_row(1, Some("t1"), Some(r#"{"number":1}"#))];
    let store = FakeStore::new("t1", Some(Vec::new()), rows, Vec::new());
    let removed = delete_team_model_alias("1", &store).await.unwrap();
    assert!(removed.is_empty());
    assert_eq!(
        store.remaining_aliases(),
        vec![Some(aliases(r#"{"number":1}"#))]
    );
}

#[tokio::test]
async fn a_failed_alias_write_does_not_skip_the_next_row() {
    let rows = vec![
        alias_row(1, Some("t1"), Some(r#"{"first":"old"}"#)),
        alias_row(2, Some("t1"), Some(r#"{"second":"old"}"#)),
    ];
    let store = FakeStore::new("t1", Some(Vec::new()), rows, Vec::new());
    let failing = FailingAliasRows {
        store,
        failing_id: 1,
    };
    let error = delete_team_model_alias("old", &failing).await.unwrap_err();
    assert!(matches!(error, CleanupError::Aliases { id: 1 }));
    assert_eq!(
        failing.store.remaining_aliases(),
        vec![Some(aliases(r#"{"first":"old"}"#)), Some(aliases("{}"))]
    );
}

struct FailingAliasRows {
    store: FakeStore,
    failing_id: i64,
}

#[async_trait]
impl AliasRows for FailingAliasRows {
    async fn alias_rows(&self) -> Result<Vec<AliasRow>, CleanupError> {
        self.store.alias_rows().await
    }

    async fn save_aliases(&self, id: i64, aliases: &AliasMap) -> Result<(), CleanupError> {
        if id == self.failing_id {
            return Err(CleanupError::Aliases { id });
        }
        self.store.save_aliases(id, aliases).await
    }
}
