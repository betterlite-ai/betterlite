#![cfg(test)]

use super::errors::DbError;
use super::session::{DbSession, PostgresStore};
use betterlite_postgres_fixture::connection::PostgresFixture;
use clap::Parser;
use deadpool_postgres::{Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::NoTls;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

async fn database() -> (PostgresFixture, PostgresStore) {
    let url = Environment::parse_from(["postgres-tests"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(url.as_ref())
        .resolve()
        .await
        .unwrap();
    let pool = pool(&fixture.options.url);
    (fixture, PostgresStore { pool: Some(pool) })
}

#[tokio::test]
async fn missing_client_exact_message() {
    let store = PostgresStore { pool: None };
    let error = store.session().await.err().unwrap();
    assert!(matches!(error, DbError::NoDbConnected));
    assert_eq!(
        error.to_string(),
        "No DB Connected. See - https://docs.litellm.ai/docs/proxy/virtual_keys"
    );
}

#[tokio::test]
async fn postgres_drop_rolls_back() {
    let (fixture, store) = database().await;
    let pool = store.pool.as_ref().unwrap();
    let mut client = store.session().await.unwrap();
    let transaction: DbSession<'_> = client.transaction().await.unwrap();
    transaction
        .query_one("SELECT txid_current()", &[])
        .await
        .unwrap();
    drop(transaction);
    drop(client);
    let connection = pool.get().await.unwrap();
    let row = connection
        .query_one("SELECT txid_current_if_assigned()", &[])
        .await
        .unwrap();
    assert_eq!(row.get::<_, Option<i64>>(0), None);
    drop(connection);
    pool.close();
    fixture.close_async().await.unwrap();
}

use super::errors::{backend, is_unique_violation};
use super::repository::Repository;
use super::rows::{FindMany, Key, KeyAction, KeyColumn, Query, RowCodec, Table};
use rstest::rstest;
use std::convert::Infallible;
use std::error::Error;
use std::future::{Future as _, pending, poll_fn};
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::timeout;
use tokio_postgres::{Row, error::SqlState};

#[derive(Default)]
struct Records {
    sql: &'static str,
    options: (bool, bool, Option<i64>, Option<i64>),
}
impl RowCodec for Records {
    type Model = String;
    type Error = DecodeError;
    fn decode(&self, row: &Row) -> Result<Option<String>, DecodeError> {
        match row
            .try_get::<_, &str>(0)
            .map_err(|driver| DecodeError::Column {
                source: super::diagnostics::DriverError { driver },
            })? {
            "invalid" => Err(DecodeError::Invalid),
            "null" => Ok(None),
            value => Ok(Some(value.to_owned())),
        }
    }
}
impl Table for Records {
    type Column = ();
    type Data = String;
    type Filter = ();
    type Order = ();
    fn keyed<'query>(
        &'query self,
        action: KeyAction<'query, String>,
        key: Key<'query, ()>,
    ) -> Query<'query> {
        let mut parameters = vec![key.value];
        if let KeyAction::Update(value) = &action {
            parameters.push(*value);
        }
        let sql = match (action, key.column) {
            (KeyAction::Find, KeyColumn::Id) => "SELECT id FROM records WHERE id=$1",
            (KeyAction::Find, KeyColumn::Alternate(())) => "SELECT id FROM records WHERE alias=$1",
            (KeyAction::Update(_), KeyColumn::Id) => {
                "UPDATE records SET alias=$2 WHERE id=$1 RETURNING id"
            }
            (KeyAction::Update(_), KeyColumn::Alternate(())) => {
                "UPDATE records SET alias=$2 WHERE alias=$1 RETURNING id"
            }
            (KeyAction::Delete, KeyColumn::Id) => "DELETE FROM records WHERE id=$1 RETURNING id",
            (KeyAction::Delete, KeyColumn::Alternate(())) => {
                "DELETE FROM records WHERE alias=$1 RETURNING id"
            }
        };
        Query { sql, parameters }
    }
    fn insert<'query>(&'query self, data: &'query String) -> Query<'query> {
        Query {
            sql: self.sql,
            parameters: vec![data],
        }
    }
    fn list<'query>(&'query self, options: FindMany<'query, (), ()>) -> Query<'query> {
        assert_eq!(
            (
                options.filter.is_some(),
                options.order.is_some(),
                options.skip,
                options.take
            ),
            self.options
        );
        Query {
            sql: self.sql,
            parameters: vec![],
        }
    }
    fn count<'query>(&'query self, filter: Option<&'query [()]>) -> Query<'query> {
        assert_eq!(filter.is_some(), self.options.0);
        Query {
            sql: "SELECT count(*) FROM records",
            parameters: vec![],
        }
    }
    fn membership(&self, _column: KeyColumn<()>) -> &str {
        self.sql
    }
}

#[tokio::test]
async fn optional_client_access_fails() {
    let repository = Repository::<deadpool_postgres::Client, _> {
        client: None,
        table: Records::default(),
    };
    assert!(matches!(
        repository
            .find_many_in(KeyColumn::Id, Vec::<String>::new())
            .await,
        Err(DbError::NoDbConnected)
    ));
    assert!(matches!(
        repository.create(&"key".to_owned()).await,
        Err(DbError::NoDbConnected)
    ));
}

#[derive(Debug, thiserror::Error)]
enum DecodeError {
    #[error("invalid record")]
    Invalid,
    #[error("invalid column")]
    Column {
        #[source]
        source: super::diagnostics::DriverError,
    },
}

async fn records() -> (PostgresFixture, PostgresStore, deadpool_postgres::Client) {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    client.batch_execute("CREATE TABLE records(id TEXT PRIMARY KEY, alias TEXT UNIQUE); INSERT INTO records VALUES ('key','alias'), ('invalid','bad')").await.unwrap();
    (fixture, store, client)
}

#[rstest]
#[case::find_id(KeyAction::Find, KeyColumn::default(), "key")]
#[case::find_alternate(KeyAction::Find, KeyColumn::Alternate(()), "alias")]
#[case::update_id(KeyAction::Update(&"changed".to_owned()), KeyColumn::default(), "key")]
#[case::update_alternate(KeyAction::Update(&"changed".to_owned()), KeyColumn::Alternate(()), "alias")]
#[case::delete_id(KeyAction::Delete, KeyColumn::default(), "key")]
#[case::delete_alternate(KeyAction::Delete, KeyColumn::Alternate(()), "alias")]
#[tokio::test]
async fn postgres_default_and_alt_key(
    #[case] action: KeyAction<'_, String>,
    #[case] column: KeyColumn<()>,
    #[case] value: &str,
) {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records::default(),
    };
    let result = repository
        .by_key(
            action,
            Key {
                column,
                value: &value,
            },
        )
        .await
        .unwrap();
    assert_eq!(result, Some("key".to_owned()));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_exists_does_not_decode() {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records::default(),
    };
    assert!(
        repository
            .exists(Key {
                column: KeyColumn::Id,
                value: &"invalid"
            })
            .await
            .unwrap()
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_decode_cause_retained() {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records::default(),
    };
    let error = repository
        .by_key(
            KeyAction::Find,
            Key {
                column: KeyColumn::Id,
                value: &"invalid",
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(
        &error,
        DbError::InvalidRecord {
            source: DecodeError::Invalid
        }
    ));
    assert!(matches!(
        error.source().unwrap().downcast_ref::<DecodeError>(),
        Some(DecodeError::Invalid)
    ));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_backend_error_not_none() {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records::default(),
    };
    let conflict = "bad".to_owned();
    let result = repository
        .by_key(
            KeyAction::Update(&conflict),
            Key {
                column: KeyColumn::Id,
                value: &"key",
            },
        )
        .await;
    assert!(matches!(result, Err(DbError::UniqueViolation { .. })));
    finish(fixture, store, client).await;
}

#[rstest]
#[case::find(KeyAction::Find)]
#[case::update(KeyAction::Update(&"changed".to_owned()))]
#[case::delete(KeyAction::Delete)]
#[tokio::test]
async fn postgres_missing_result_none(#[case] action: KeyAction<'_, String>) {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records::default(),
    };
    let result = repository
        .by_key(
            action,
            Key {
                column: KeyColumn::Id,
                value: &"missing",
            },
        )
        .await
        .unwrap();
    assert_eq!(result, None);
    finish(fixture, store, client).await;
}

#[rstest]
#[case::empty_where_order_absent(None)]
#[case::skip_take_zero_preserved(Some(0))]
#[tokio::test]
async fn postgres_list_defaults(#[case] pagination: Option<i64>) {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records {
            sql: "SELECT unnest(ARRAY['second','null','first'])",
            options: (false, false, pagination, pagination),
        },
    };
    let options = FindMany {
        filter: Some(&[]),
        order: Some(&[]),
        skip: pagination,
        take: pagination,
    };
    assert_eq!(
        repository.find_many(options).await.unwrap(),
        vec!["second", "first"]
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_count_filter_preserved() {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records {
            sql: "",
            options: (true, false, None, None),
        },
    };
    assert_eq!(repository.count(Some(&[])).await.unwrap(), 2);
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_invalid_list_fails() {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records {
            sql: "SELECT 'invalid'::text",
            options: (false, false, None, None),
        },
    };
    assert!(matches!(
        repository.find_many(FindMany::default()).await,
        Err(DbError::InvalidRecord {
            source: DecodeError::Invalid
        })
    ));
    finish(fixture, store, client).await;
}

#[rstest]
#[case::no_row("SELECT $1::text WHERE false", "key")]
#[case::no_model("SELECT $1::text", "null")]
#[tokio::test]
async fn postgres_create_null_invariant(#[case] sql: &'static str, #[case] value: &str) {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records {
            sql,
            options: (false, false, None, None),
        },
    };
    assert!(matches!(
        repository.create(&value.to_owned()).await,
        Err(DbError::CreateNull)
    ));
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_empty_no_statement() {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records::default(),
    };
    assert!(
        repository
            .find_many_in(KeyColumn::Id, Vec::<String>::new())
            .await
            .unwrap()
            .is_empty()
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_stable_input_dedup() {
    let (fixture, store, client) = records().await;
    let repository = Repository {
        client: Some(&client),
        table: Records {
            sql: "SELECT unnest($1::text[])",
            options: (false, false, None, None),
        },
    };
    assert_eq!(
        repository
            .find_many_in(KeyColumn::Id, vec!["b", "a", "b"])
            .await
            .unwrap(),
        vec!["b", "a"]
    );
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_bound_array_literal() {
    let (fixture, store, client) = records().await;
    let literal = "'); DROP TABLE records; --";
    client
        .execute("INSERT INTO records(id) VALUES ($1)", &[&literal])
        .await
        .unwrap();
    let repository = Repository {
        client: Some(&client),
        table: Records {
            sql: "SELECT id FROM records WHERE id = ANY($1)",
            options: (false, false, None, None),
        },
    };
    assert_eq!(
        repository
            .find_many_in(KeyColumn::Id, vec![literal, literal])
            .await
            .unwrap(),
        vec![literal]
    );
    let mut ids: Vec<String> = (0_u32..40_000).map(|index| index.to_string()).collect();
    ids.push("key".to_owned());
    assert_eq!(
        repository.find_many_in(KeyColumn::Id, ids).await.unwrap(),
        vec!["key"]
    );
    finish(fixture, store, client).await;
}

async fn constraints() -> (PostgresFixture, PostgresStore, deadpool_postgres::Client) {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    client.batch_execute("CREATE TABLE records(id TEXT PRIMARY KEY, parent TEXT REFERENCES records(id)); INSERT INTO records VALUES ('secret', NULL)").await.unwrap();
    (fixture, store, client)
}

#[rstest]
#[case::unique_sqlstate_23505(
    "INSERT INTO records VALUES ('secret', NULL)",
    SqlState::UNIQUE_VIOLATION,
    Some("records_pkey")
)]
#[case::foreign_key_not_unique(
    "INSERT INTO records VALUES ('other', 'secret-missing')",
    SqlState::FOREIGN_KEY_VIOLATION,
    Some("records_parent_fkey")
)]
#[case::not_null_not_unique(
    "INSERT INTO records VALUES (NULL, 'secret')",
    SqlState::NOT_NULL_VIOLATION,
    None
)]
#[tokio::test]
async fn postgres_constraint_identity(
    #[case] sql: &str,
    #[case] state: SqlState,
    #[case] constraint: Option<&str>,
) {
    let (fixture, store, client) = constraints().await;
    let error: DbError = backend(client.batch_execute(sql).await.unwrap_err());
    let unique = state == SqlState::UNIQUE_VIOLATION;
    assert_eq!(is_unique_violation(&error), unique);
    match &error {
        DbError::UniqueViolation {
            constraint: actual,
            source,
        } => {
            assert_eq!(actual.as_deref(), constraint);
            assert_eq!(source.driver.code(), Some(&state));
        }
        DbError::SqlState {
            code,
            constraint: actual,
            source,
        } => {
            assert_eq!(*code, state);
            assert_eq!(actual.as_deref(), constraint);
            assert_eq!(source.driver.code(), Some(&state));
        }
        DbError::NoDbConnected
        | DbError::Pool { .. }
        | DbError::Backend { .. }
        | DbError::InvalidRecord { .. }
        | DbError::CreateNull => panic!("unexpected error: {error}"),
    }
    finish(fixture, store, client).await;
}

#[rstest]
#[case::unique("INSERT INTO records VALUES ('secret', NULL)")]
#[case::foreign_key("INSERT INTO records VALUES ('other', 'secret-missing')")]
#[case::not_null("INSERT INTO records VALUES (NULL, 'secret')")]
#[tokio::test]
async fn postgres_secrets_absent(#[case] sql: &str) {
    let (fixture, store, client) = constraints().await;
    let error: DbError = backend(client.batch_execute(sql).await.unwrap_err());
    assert_safe_chain(&error);
    finish(fixture, store, client).await;
}

fn assert_safe_chain(error: &dyn Error) {
    let mut cause = Some(error);
    while let Some(current) = cause {
        assert!(!format!("{current} {current:?}").contains("secret"));
        cause = current.source();
    }
}

#[test]
fn backend_secrets_absent() {
    let driver = "secret=credential"
        .parse::<tokio_postgres::Config>()
        .unwrap_err();
    assert!(format!("{driver:?}").contains("secret"));
    let error: DbError = backend(driver);
    assert!(matches!(&error, DbError::Backend { .. }));
    assert_safe_chain(&error);
}

#[rstest]
#[case::explicit_commit(true)]
#[case::explicit_error_rollback(false)]
#[tokio::test]
async fn postgres_transactions(#[case] commit: bool) {
    let (fixture, store) = database().await;
    let mut client = store.session().await.unwrap();
    client
        .batch_execute("CREATE TABLE records(id TEXT PRIMARY KEY)")
        .await
        .unwrap();
    let transaction: DbSession<'_> = client.transaction().await.unwrap();
    let table = Records {
        sql: "INSERT INTO records VALUES ($1) RETURNING id",
        options: (false, false, None, None),
    };
    let repository = Repository {
        client: Some(&transaction),
        table,
    };
    let created = repository.create(&"first".to_owned()).await.unwrap();
    assert_eq!(created, "first");
    assert_eq!(repository.count(None).await.unwrap(), 1);
    if commit {
        transaction.commit().await.unwrap();
    } else {
        assert!(is_unique_violation(
            &repository.create(&"first".to_owned()).await.unwrap_err()
        ));
        transaction.rollback().await.unwrap();
    }
    drop(client);
    let reader = store.session().await.unwrap();
    let count = reader
        .query_one("SELECT count(*) FROM records", &[])
        .await
        .unwrap();
    assert_eq!(count.get::<_, i64>(0), i64::from(commit));
    finish(fixture, store, reader).await;
}

#[tokio::test]
async fn postgres_cancel_and_autocommit() {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    client
        .batch_execute(
            "CREATE TABLE records(id TEXT PRIMARY KEY); INSERT INTO records VALUES ('committed')",
        )
        .await
        .unwrap();
    drop(client);
    cancel_write(&store).await;
    let reader = timeout(Duration::from_secs(5), store.session())
        .await
        .unwrap()
        .unwrap();
    let error = reader
        .execute("INSERT INTO records VALUES ('committed')", &[])
        .await
        .unwrap_err();
    assert!(is_unique_violation(&backend::<Infallible>(error)));
    let rows = reader.query("SELECT id FROM records", &[]).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows.first().unwrap().get::<_, String>(0), "committed");
    finish(fixture, store, reader).await;
}

#[tokio::test]
async fn postgres_fixture_contracts() {
    use betterlite_postgres_fixture::connection::{FixtureSource, select_source};
    assert!(matches!(
        select_source(None).unwrap(),
        FixtureSource::Testcontainers
    ));
    let (fixture, store) = database().await;
    // Start from the server URL, since a new fixture must not inherit the first schema's search path.
    let raw = fixture.options.url.expose_secret();
    let server = SecretString::from(raw.split('?').next().unwrap().to_owned());
    let second = PostgresFixture::provision_schema(Some(&server))
        .resolve()
        .await
        .unwrap();
    assert!(second.container.is_none());
    assert_ne!(
        fixture.options.config.get_options(),
        second.options.config.get_options()
    );
    let client = store.session().await.unwrap();
    client
        .batch_execute("CREATE TABLE records(id TEXT)")
        .await
        .unwrap();
    let other_pool = pool(&second.options.url);
    let other = other_pool.get().await.unwrap();
    let row = other
        .query_one("SELECT to_regclass('records')::text", &[])
        .await
        .unwrap();
    assert!(row.get::<_, Option<String>>(0).is_none());
    drop(other);
    other_pool.close();
    second.close_async().await.unwrap();
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_single_array() {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    let repository = Repository {
        client: Some(&client),
        table: Records {
            sql: "SELECT cardinality($1::text[])::text",
            options: (false, false, None, None),
        },
    };
    let values: Vec<String> = (0_i32..40_000_i32).map(|value| value.to_string()).collect();
    assert_eq!(
        repository
            .find_many_in(KeyColumn::Id, values)
            .await
            .unwrap(),
        vec!["40000"]
    );
    finish(fixture, store, client).await;
}

fn pool(url: &SecretString) -> Pool {
    let config = url.expose_secret().parse().unwrap();
    let manager = Manager::new(config, NoTls);
    Pool::builder(manager).max_size(1).build().unwrap()
}

async fn finish(fixture: PostgresFixture, store: PostgresStore, client: deadpool_postgres::Client) {
    drop(client);
    store.pool.unwrap().close();
    fixture.close_async().await.unwrap();
}

async fn cancel_write(store: &PostgresStore) {
    let (sender, mut receiver) = oneshot::channel();
    let mut operation = Box::pin(async {
        let mut connection = store.session().await.unwrap();
        let transaction = connection.transaction().await.unwrap();
        transaction
            .execute("INSERT INTO records VALUES ('cancelled')", &[])
            .await
            .unwrap();
        sender.send(()).unwrap();
        pending::<()>().await;
    });
    timeout(
        Duration::from_secs(5),
        poll_fn(|context| {
            assert!(operation.as_mut().poll(context).is_pending());
            Pin::new(&mut receiver).poll(context)
        }),
    )
    .await
    .unwrap()
    .unwrap();
    drop(operation);
}

#[test]
fn find_many_defaults_are_unset() {
    let options: FindMany<'_, Infallible, Infallible> = FindMany::default();
    assert_eq!(
        (options.filter, options.order, options.skip, options.take),
        (None, None, None, None)
    );
}

#[test]
fn pool_secrets_absent() {
    let driver = "secret=credential"
        .parse::<tokio_postgres::Config>()
        .unwrap_err();
    assert!(format!("{driver:?}").contains("secret"));
    let error: DbError = DbError::Pool {
        source: super::diagnostics::PoolFailure {
            driver: deadpool_postgres::PoolError::Backend(driver),
        },
    };
    assert_safe_chain(&error);
}
