use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use betterlite_db::database_failures::deadline::DbLookupError;
use betterlite_db::database_faults::ErrorType;
use rstest::rstest;

use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::table::{AuthTable, Create, FindMany, FindOne, Presence, Update};

use super::support::{
    Fields, Harness, Mode, RecordedCall, assert_deadline, recording_repo, recording_repo_with,
    rich_row,
};

fn filter() -> Fields {
    Fields::from([("id".to_owned(), "row-1".to_owned())])
}

#[tokio::test(start_paused = true)]
async fn unique_first_many_use_deadline() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Pending);
    let table = AuthReadAdapter::Dictable.open(
        Arc::clone(&repo),
        "dictable".to_owned(),
        harness.context(5.0),
    );
    let unique = table
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("a stalled unique read is a deadline");
    let first = table
        .find_first(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("a stalled first read is a deadline");
    let many = table
        .find_many(FindMany {
            filter: Presence::Value(filter()),
            include: Presence::Omitted,
            take: Presence::Omitted,
        })
        .await
        .expect_err("a stalled many read is a deadline");
    assert_deadline(unique, "dictable", 5.0);
    assert_deadline(first, "dictable", 5.0);
    assert_deadline(many, "dictable", 5.0);
    assert_eq!(repo.calls().len(), 3);
    assert!(harness.tracker.stalled_within(f64::INFINITY));
    harness.clock.advance(6);
    assert!(!harness.tracker.stalled_within(5.0));
    assert!(harness.tracker.stalled_within(7.0));
    harness.drain().await;
}

#[tokio::test(start_paused = true)]
async fn explicit_lookup_label_forwarded() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Pending);
    let dictable = AuthReadAdapter::Dictable.open(
        Arc::clone(&repo),
        "custom_lookup".to_owned(),
        harness.context(1.0),
    );
    let model_dump = AuthReadAdapter::ModelDump.open(
        Arc::clone(&repo),
        "model_dump_label".to_owned(),
        harness.context(1.0),
    );
    let dictable_error = dictable
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Null,
        })
        .await
        .expect_err("the dictable lookup is bounded");
    let model_dump_error = model_dump
        .find_first(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("the model_dump lookup is bounded");
    assert_deadline(dictable_error, "custom_lookup", 1.0);
    assert_deadline(model_dump_error, "model_dump_label", 1.0);
    harness.drain().await;
}

#[rstest]
#[case::jwt_key_mapping(AuthReadAdapter::JwtKeyMapping, "jwt_key_mapping")]
#[case::team(AuthReadAdapter::Team, "team")]
#[case::vector_store(AuthReadAdapter::VectorStore, "vector_store")]
#[case::user(AuthReadAdapter::User, "user")]
#[case::object_permission(AuthReadAdapter::ObjectPermission, "object_permission")]
#[tokio::test(start_paused = true)]
async fn fixed_lookup_labels(#[case] adapter: AuthReadAdapter, #[case] label: &str) {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Pending);
    let table = adapter.open(
        Arc::clone(&repo),
        "ignored_caller_label".to_owned(),
        harness.context(2.0),
    );
    let error = table
        .find_many(FindMany {
            filter: Presence::Omitted,
            include: Presence::Omitted,
            take: Presence::Omitted,
        })
        .await
        .expect_err("the fixed-label adapter is bounded");
    assert_deadline(error, label, 2.0);
    harness.drain().await;
}

#[tokio::test]
async fn kwargs_omission_vs_null() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Row);
    let table = AuthReadAdapter::Tag.open(Arc::clone(&repo), String::new(), harness.context(5.0));
    let include = Fields::from([("litellm_budget_table".to_owned(), "true".to_owned())]);
    let unique = table
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect("the tag read succeeds");
    let first = table
        .find_first(FindOne {
            filter: filter(),
            include: Presence::Null,
        })
        .await
        .expect("the tag read succeeds");
    let omitted = table
        .find_many(FindMany {
            filter: Presence::Omitted,
            include: Presence::Omitted,
            take: Presence::Omitted,
        })
        .await
        .expect("the tag read succeeds");
    let null = table
        .find_many(FindMany {
            filter: Presence::Null,
            include: Presence::Null,
            take: Presence::Null,
        })
        .await
        .expect("the tag read succeeds");
    let values = table
        .find_many(FindMany {
            filter: Presence::Value(filter()),
            include: Presence::Value(include.clone()),
            take: Presence::Value(0),
        })
        .await
        .expect("the tag read succeeds");
    assert!(unique.is_some());
    assert!(first.is_some());
    assert_eq!(omitted.len(), 3);
    assert_eq!(null.len(), 3);
    assert_eq!(values.len(), 3);
    assert_eq!(
        repo.calls(),
        vec![
            RecordedCall::FindUnique {
                filter: filter(),
                include: Presence::Omitted,
            },
            RecordedCall::FindFirst {
                filter: filter(),
                include: Presence::Null,
            },
            RecordedCall::FindMany {
                filter: Presence::Omitted,
                include: Presence::Omitted,
                take: Presence::Omitted,
            },
            RecordedCall::FindMany {
                filter: Presence::Null,
                include: Presence::Null,
                take: Presence::Null,
            },
            RecordedCall::FindMany {
                filter: Presence::Value(filter()),
                include: Presence::Value(include),
                take: Presence::Value(0),
            },
        ]
    );
}

#[tokio::test]
async fn empty_filters_and_zero_take_kept() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Row);
    let table =
        AuthReadAdapter::EndUser.open(Arc::clone(&repo), String::new(), harness.context(5.0));
    let rows = table
        .find_many(FindMany {
            filter: Presence::Value(Fields::new()),
            include: Presence::Value(Fields::new()),
            take: Presence::Value(0),
        })
        .await
        .expect("the end-user read succeeds");
    assert_eq!(rows.len(), 3);
    assert_eq!(
        repo.calls(),
        vec![RecordedCall::FindMany {
            filter: Presence::Value(Fields::new()),
            include: Presence::Value(Fields::new()),
            take: Presence::Value(0),
        }]
    );
}

#[tokio::test]
async fn returned_rows_order_and_duplicates() {
    let harness = Harness::new();
    let listed = vec![rich_row("second"), rich_row("first"), rich_row("second")];
    let repo = recording_repo_with(Mode::Row, listed.clone());
    let table = AuthReadAdapter::User.open(Arc::clone(&repo), String::new(), harness.context(5.0));
    let found = table
        .find_many(FindMany {
            filter: Presence::Omitted,
            include: Presence::Omitted,
            take: Presence::Omitted,
        })
        .await
        .expect("the user read succeeds");
    assert_eq!(found, listed);
}

#[tokio::test(start_paused = true)]
async fn deadline_error_not_missing_row() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Pending);
    let table = AuthReadAdapter::Team.open(Arc::clone(&repo), String::new(), harness.context(5.0));
    let error = table
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("a stalled read is an error, not a missing row");
    assert_deadline(error, "team", 5.0);
    harness.drain().await;
}

#[tokio::test]
async fn repository_error_cause_kept() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Error);
    let table = AuthReadAdapter::ObjectPermission.open(
        Arc::clone(&repo),
        String::new(),
        harness.context(5.0),
    );
    let error = table
        .find_many(FindMany {
            filter: Presence::Omitted,
            include: Presence::Omitted,
            take: Presence::Omitted,
        })
        .await
        .expect_err("the repository failure surfaces unchanged");
    let DbLookupError::Database(database) = error else {
        panic!("the repository error must not become a deadline, got {error:?}");
    };
    assert_eq!(database.error_type, ErrorType::Read);
    assert_eq!(database.message, "the membership read failed");
    assert_eq!(database.code.as_deref(), Some("XX000"));
    let cause = std::error::Error::source(&database).expect("the nested cause is kept");
    assert_eq!(cause.to_string(), "the pool refused the connection");
}

#[tokio::test]
async fn read_cancel_propagates() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Pending);
    let table = AuthReadAdapter::User.open(Arc::clone(&repo), String::new(), harness.context(60.0));
    {
        let mut read = Box::pin(table.find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        }));
        std::future::poll_fn(|poll_context| {
            assert!(
                read.as_mut().poll(poll_context).is_pending(),
                "the read waits on the pending repository"
            );
            std::task::Poll::Ready(())
        })
        .await;
        tokio::task::yield_now().await;
        assert!(!repo.dropped());
    }
    tokio::task::yield_now().await;
    assert!(repo.dropped());
    harness.drain().await;
}

#[tokio::test(start_paused = true)]
async fn writes_do_not_use_read_deadline() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Delayed);
    let table = AuthReadAdapter::User.open(Arc::clone(&repo), String::new(), harness.context(0.0));
    let data = Fields::from([("spend".to_owned(), "1.25".to_owned())]);
    let updated = table
        .update(Update {
            filter: filter(),
            data: data.clone(),
        })
        .await
        .expect("a write is not bounded by the read deadline");
    let created = table
        .create(Create {
            data: data.clone(),
            include: Presence::Value(Fields::new()),
        })
        .await
        .expect("a write is not bounded by the read deadline");
    assert_eq!(updated, Some(rich_row("user-1")));
    assert_eq!(created, rich_row("user-1"));
    assert_eq!(
        repo.calls(),
        vec![
            RecordedCall::Update {
                filter: filter(),
                data: data.clone(),
            },
            RecordedCall::Create {
                data,
                include: Presence::Value(Fields::new()),
            },
        ]
    );
    harness.drain().await;
}

#[rstest]
#[case::tag(AuthReadAdapter::Tag)]
#[case::end_user(AuthReadAdapter::EndUser)]
#[case::model_access_group_budget(AuthReadAdapter::ModelAccessGroupBudget)]
#[tokio::test(start_paused = true)]
async fn tag_end_user_group_adapters_unwrapped(#[case] adapter: AuthReadAdapter) {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Pending);
    let table = adapter.open(Arc::clone(&repo), String::new(), harness.context(5.0));
    let read = tokio::time::timeout(
        Duration::from_secs(30),
        table.find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        }),
    )
    .await;
    assert!(
        read.is_err(),
        "an unwrapped adapter applies no read deadline"
    );
    assert_eq!(repo.calls().len(), 1);
    harness.drain().await;
}

#[tokio::test(start_paused = true)]
async fn dictable_end_user_still_bounded() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Pending);
    let table = AuthReadAdapter::Dictable.open(
        Arc::clone(&repo),
        "end_user".to_owned(),
        harness.context(5.0),
    );
    let error = table
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("the adapter choice bounds the end-user lookup");
    assert_deadline(error, "end_user", 5.0);
    harness.drain().await;
}
