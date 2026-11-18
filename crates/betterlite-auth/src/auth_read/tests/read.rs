use std::error::Error;
use std::future::{Future as _, poll_fn};
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;

use betterlite_db::database_failures::deadline::DbLookupError;
use rstest::rstest;
use tokio::task::yield_now;
use tokio::time::timeout;

use crate::auth_read::adapter::AuthReadAdapter;
use crate::auth_read::repository::RepositoryTable;
use crate::auth_read::table::{
    AuthReadError, AuthTable as _, Create, FindMany, FindOne, Presence, Update,
};
use crate::auth_read::user_rows::{UserCreate, UserEmailFilter, UserRows};

use super::support::{
    Fields, Harness, Mode, RecordedCall, assert_connect, assert_deadline, native_key,
    recording_repo, recording_repo_with, rich_row,
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

struct Requests {
    unique: FindOne<Fields, Fields>,
    first: FindOne<Fields, Fields>,
    omitted: FindMany<Fields, Fields>,
    null: FindMany<Fields, Fields>,
    values: FindMany<Fields, Fields>,
}

fn requests() -> Requests {
    let include = Fields::from([("litellm_budget_table".to_owned(), "true".to_owned())]);
    Requests {
        unique: FindOne {
            filter: filter(),
            include: Presence::Omitted,
        },
        first: FindOne {
            filter: filter(),
            include: Presence::Null,
        },
        omitted: FindMany {
            filter: Presence::Omitted,
            include: Presence::Omitted,
            take: Presence::Omitted,
        },
        null: FindMany {
            filter: Presence::Null,
            include: Presence::Null,
            take: Presence::Null,
        },
        values: FindMany {
            filter: Presence::Value(filter()),
            include: Presence::Value(include),
            take: Presence::Value(0),
        },
    }
}

#[tokio::test]
async fn kwargs_omission_vs_null() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Row);
    let table = AuthReadAdapter::Tag.open(Arc::clone(&repo), String::new(), harness.context(5.0));
    let requests = requests();
    let unique = table
        .find_unique(requests.unique.clone())
        .await
        .expect("the tag read succeeds");
    let first = table
        .find_first(requests.first.clone())
        .await
        .expect("the tag read succeeds");
    let omitted = table
        .find_many(requests.omitted.clone())
        .await
        .expect("the tag read succeeds");
    let null = table
        .find_many(requests.null.clone())
        .await
        .expect("the tag read succeeds");
    let values = table
        .find_many(requests.values.clone())
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
            RecordedCall::FindUnique(requests.unique),
            RecordedCall::FindFirst(requests.first),
            RecordedCall::FindMany(requests.omitted),
            RecordedCall::FindMany(requests.null),
            RecordedCall::FindMany(requests.values),
        ]
    );
}

#[tokio::test]
async fn empty_filters_and_zero_take_kept() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Row);
    let table =
        AuthReadAdapter::EndUser.open(Arc::clone(&repo), String::new(), harness.context(5.0));
    let request = FindMany {
        filter: Presence::Value(Fields::new()),
        include: Presence::Value(Fields::new()),
        take: Presence::Value(0),
    };
    let rows = table
        .find_many(request.clone())
        .await
        .expect("the end-user read succeeds");
    assert_eq!(rows.len(), 3);
    assert_eq!(repo.calls(), vec![RecordedCall::FindMany(request)]);
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
    let AuthReadError::Repository(fault) = error else {
        panic!("the repository error must not become a deadline, got {error:?}");
    };
    assert_eq!(fault.to_string(), "invalid database record");
    let decode = Error::source(&fault).expect("the decode failure stays attached");
    assert_eq!(
        decode.to_string(),
        "the stored user record is not a readable row"
    );
    let reason = Error::source(decode).expect("the decode reason stays attached");
    assert_eq!(
        reason.to_string(),
        "the stored user record needs to be a JSON object"
    );
}

#[tokio::test]
async fn read_cancel_propagates() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Pending);
    let table = AuthReadAdapter::User.open(Arc::clone(&repo), String::new(), harness.context(60.0));
    let mut read = Box::pin(table.find_unique(FindOne {
        filter: filter(),
        include: Presence::Omitted,
    }));
    poll_fn(|poll_context| {
        assert!(
            read.as_mut().poll(poll_context).is_pending(),
            "the read waits on the pending repository"
        );
        Poll::Ready(())
    })
    .await;
    yield_now().await;
    assert!(!repo.dropped());
    drop(read);
    yield_now().await;
    assert!(repo.dropped());
    harness.drain().await;
}

#[tokio::test(start_paused = true)]
async fn writes_do_not_use_read_deadline() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Delayed);
    let table = AuthReadAdapter::User.open(Arc::clone(&repo), String::new(), harness.context(0.0));
    let data = Fields::from([("spend".to_owned(), "1.25".to_owned())]);
    let update_request = Update {
        filter: filter(),
        data: data.clone(),
    };
    let create_request = Create {
        data,
        include: Presence::Value(Fields::new()),
    };
    let updated = table
        .update(update_request.clone())
        .await
        .expect("a write is not bounded by the read deadline");
    let created = table
        .create(create_request.clone())
        .await
        .expect("a write is not bounded by the read deadline");
    assert_eq!(updated, Some(rich_row("user-1")));
    assert_eq!(created, rich_row("user-1"));
    assert_eq!(
        repo.calls(),
        vec![
            RecordedCall::Update(update_request),
            RecordedCall::Create(create_request),
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
    let read = timeout(
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

#[test]
fn presence_distinguishes_omitted_null_and_value() {
    let present: Presence<i64> = Presence::Value(0);
    assert_eq!(present.value(), Some(&0));
    assert_eq!(Presence::<i64>::Null.value(), None);
    assert_eq!(Presence::<i64>::Omitted.value(), None);
}

#[tokio::test]
async fn repository_composes_with_native_table() {
    let harness = Harness::new();
    let repository: RepositoryTable<deadpool_postgres::Client, _> = RepositoryTable {
        client: None,
        table: UserRows,
    };
    let table = AuthReadAdapter::Team.open(
        Arc::new(repository),
        "team".to_owned(),
        harness.context(5.0),
    );
    assert_connect(
        table
            .update(Update {
                filter: native_key(),
                data: UserCreate::default(),
            })
            .await,
    );
    assert_connect(
        table
            .create(Create {
                data: UserCreate::default(),
                include: Presence::Omitted,
            })
            .await,
    );
    harness.drain().await;
}

#[tokio::test]
async fn repository_composes_with_native_reads() {
    let harness = Harness::new();
    let repository: RepositoryTable<deadpool_postgres::Client, _> = RepositoryTable {
        client: None,
        table: UserRows,
    };
    let table = AuthReadAdapter::Team.open(
        Arc::new(repository),
        "team".to_owned(),
        harness.context(5.0),
    );
    assert_connect(
        table
            .find_unique(FindOne {
                filter: native_key(),
                include: Presence::Omitted,
            })
            .await,
    );
    assert_connect(
        table
            .find_first(FindOne {
                filter: UserEmailFilter::default(),
                include: Presence::Omitted,
            })
            .await,
    );
    assert_connect(
        table
            .find_many(FindMany {
                filter: Presence::Omitted,
                include: Presence::Omitted,
                take: Presence::Omitted,
            })
            .await,
    );
    harness.drain().await;
}

#[tokio::test(start_paused = true)]
async fn inner_lookup_timeout_is_named_deadline() {
    let harness = Harness::new();
    let repo = recording_repo(Mode::Elapsed);
    let table = AuthReadAdapter::User.open(Arc::clone(&repo), String::new(), harness.context(30.0));
    let error = table
        .find_unique(FindOne {
            filter: filter(),
            include: Presence::Omitted,
        })
        .await
        .expect_err("an elapsed inner lookup is a deadline");
    let AuthReadError::Lookup(DbLookupError::Deadline(exceeded)) = error else {
        panic!("the inner timeout must become a named deadline, got {error:?}");
    };
    assert_eq!(exceeded.lookup, "user");
    assert_eq!(exceeded.deadline_seconds.to_bits(), 30.0_f64.to_bits());
    let cause = exceeded.cause.expect("the elapsed cause stays attached");
    assert!(matches!(*cause, DbLookupError::Elapsed(_)));
    assert!(harness.tracker.stalled_within(f64::INFINITY));
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
