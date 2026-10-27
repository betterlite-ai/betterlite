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
    fn decode(&self, row: &Row) -> Result<Option<String>, DbError> {
        match row.try_get::<_, &str>(0).map_err(backend)? {
            "invalid" => Err(DbError::InvalidRecord {
                message: "invalid".to_owned(),
            }),
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
        action: KeyAction,
        key: Key<'query, ()>,
        data: Option<&'query String>,
    ) -> Query<'query> {
        let sql = match (action, key.column) {
            (KeyAction::Find, KeyColumn::Id) => "SELECT id FROM records WHERE id=$1",
            (KeyAction::Find, KeyColumn::Alternate(())) => "SELECT id FROM records WHERE alias=$1",
            (KeyAction::Update, KeyColumn::Id) => {
                "UPDATE records SET alias=$2 WHERE id=$1 RETURNING id"
            }
            (KeyAction::Update, KeyColumn::Alternate(())) => {
                "UPDATE records SET alias=$2 WHERE alias=$1 RETURNING id"
            }
            (KeyAction::Delete, KeyColumn::Id) => "DELETE FROM records WHERE id=$1 RETURNING id",
            (KeyAction::Delete, KeyColumn::Alternate(())) => {
                "DELETE FROM records WHERE alias=$1 RETURNING id"
            }
        };
        let mut parameters = vec![key.value];
        if let Some(value) = data {
            parameters.push(value);
        }
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

#[rstest]
#[case::default_id(KeyColumn::default(), "key")]
#[case::alternate_key(KeyColumn::Alternate(()), "alias")]
#[tokio::test]
async fn postgres_key_results(#[case] column: KeyColumn<()>, #[case] value: &str) {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    client.batch_execute("CREATE TABLE records(id TEXT PRIMARY KEY, alias TEXT UNIQUE); INSERT INTO records VALUES ('key','alias'), ('invalid','bad')").await.unwrap();
    let repository = Repository {
        client: Some(&client),
        table: Records::default(),
    };
    assert_eq!(
        repository
            .by_key(
                KeyAction::Find,
                Key {
                    column,
                    value: &value
                },
                None
            )
            .await
            .unwrap(),
        Some("key".to_owned())
    );
    assert!(
        repository
            .exists(Key {
                column: KeyColumn::Id,
                value: &"invalid"
            })
            .await
            .unwrap()
    );
    assert!(matches!(
        repository
            .by_key(
                KeyAction::Find,
                Key {
                    column: KeyColumn::Id,
                    value: &"invalid"
                },
                None
            )
            .await,
        Err(DbError::InvalidRecord { .. })
    ));
    let conflict = "bad".to_owned();
    assert!(matches!(
        repository
            .by_key(
                KeyAction::Update,
                Key {
                    column,
                    value: &value
                },
                Some(&conflict)
            )
            .await,
        Err(DbError::UniqueViolation { .. })
    ));
    let replacement = "changed".to_owned();
    assert_eq!(
        repository
            .by_key(
                KeyAction::Update,
                Key {
                    column,
                    value: &value
                },
                Some(&replacement)
            )
            .await
            .unwrap(),
        Some("key".to_owned())
    );
    assert_eq!(
        repository
            .by_key(
                KeyAction::Delete,
                Key {
                    column: KeyColumn::Alternate(()),
                    value: &replacement
                },
                None
            )
            .await
            .unwrap(),
        Some("key".to_owned())
    );
    assert_eq!(
        repository
            .by_key(
                KeyAction::Find,
                Key {
                    column,
                    value: &value
                },
                None
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        repository
            .by_key(
                KeyAction::Update,
                Key {
                    column,
                    value: &value
                },
                Some(&replacement)
            )
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        repository
            .by_key(
                KeyAction::Delete,
                Key {
                    column,
                    value: &value
                },
                None
            )
            .await
            .unwrap(),
        None
    );
    finish(fixture, store, client).await;
}

#[rstest]
#[case::unset(None)]
#[case::zero(Some(0))]
#[tokio::test]
async fn postgres_list_defaults(#[case] pagination: Option<i64>) {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    client
        .batch_execute("CREATE TABLE records(id TEXT)")
        .await
        .unwrap();
    let table = Records {
        sql: "SELECT unnest(ARRAY['second','null','first'])",
        options: (false, false, pagination, pagination),
    };
    let mut repository = Repository {
        client: Some(&client),
        table,
    };
    assert_eq!(
        repository
            .find_many(FindMany {
                filter: Some(&[]),
                order: Some(&[]),
                skip: pagination,
                take: pagination
            })
            .await
            .unwrap(),
        vec!["second", "first"]
    );
    repository.table.options.0 = true;
    assert_eq!(repository.count(Some(&[])).await.unwrap(), 0);
    repository.table.sql = "SELECT 'invalid'::text";
    repository.table.options.0 = false;
    assert!(matches!(
        repository
            .find_many(FindMany {
                filter: None,
                order: None,
                skip: pagination,
                take: pagination
            })
            .await,
        Err(DbError::InvalidRecord { .. })
    ));
    repository.table.sql = "SELECT $1::text WHERE false";
    assert!(matches!(
        repository.create(&"key".to_owned()).await,
        Err(DbError::CreateNull)
    ));
    repository.table.sql = "SELECT $1::text";
    assert!(matches!(
        repository.create(&"null".to_owned()).await,
        Err(DbError::CreateNull)
    ));
    assert_eq!(repository.create(&"key".to_owned()).await.unwrap(), "key");
    finish(fixture, store, client).await;
}

#[tokio::test]
async fn postgres_membership() {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    let mut repository = Repository {
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
    repository.table.sql = "SELECT unnest($1::text[])";
    assert_eq!(
        repository
            .find_many_in(KeyColumn::Id, vec!["b", "a", "b"])
            .await
            .unwrap(),
        vec!["b", "a"]
    );
    client.batch_execute("CREATE TABLE records(id TEXT); INSERT INTO records VALUES ('key'), ($$'); DROP TABLE records; --$$)").await.unwrap();
    repository.table.sql = "SELECT id FROM records WHERE id = ANY($1)";
    let literal = "'); DROP TABLE records; --";
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

#[rstest]
#[case::unique(
    "INSERT INTO records VALUES ('secret', NULL)",
    SqlState::UNIQUE_VIOLATION,
    Some("records_pkey")
)]
#[case::foreign_key(
    "INSERT INTO records VALUES ('other', 'missing')",
    SqlState::FOREIGN_KEY_VIOLATION,
    Some("records_parent_fkey")
)]
#[case::not_null(
    "INSERT INTO records VALUES (NULL, NULL)",
    SqlState::NOT_NULL_VIOLATION,
    None
)]
#[tokio::test]
async fn postgres_errors(
    #[case] sql: &str,
    #[case] state: SqlState,
    #[case] constraint: Option<&str>,
) {
    let (fixture, store) = database().await;
    let client = store.session().await.unwrap();
    client.batch_execute("CREATE TABLE records(id TEXT PRIMARY KEY, parent TEXT REFERENCES records(id)); INSERT INTO records VALUES ('secret', NULL)").await.unwrap();
    let error = backend(client.batch_execute(sql).await.unwrap_err());
    assert_eq!(
        is_unique_violation(&error),
        state == SqlState::UNIQUE_VIOLATION
    );
    match &error {
        DbError::UniqueViolation {
            constraint: actual, ..
        } => assert_eq!(actual.as_deref(), constraint),
        DbError::SqlState {
            code,
            constraint: actual,
            ..
        } => {
            assert_eq!(*code, state);
            assert_eq!(actual.as_deref(), constraint);
        }
        DbError::NoDbConnected
        | DbError::Pool { .. }
        | DbError::Backend { .. }
        | DbError::InvalidRecord { .. }
        | DbError::CreateNull => panic!("unexpected error: {error}"),
    }
    assert!(!format!("{error} {error:?}").contains("secret"));
    let repository = Repository {
        client: Some(&client),
        table: Records {
            sql: "INSERT INTO records(id) VALUES ($1) RETURNING id",
            options: (false, false, None, None),
        },
    };
    assert!(matches!(
        repository.create(&"secret".to_owned()).await,
        Err(DbError::UniqueViolation { .. })
    ));
    finish(fixture, store, client).await;
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
    assert_eq!(
        repository.create(&"first".to_owned()).await.unwrap(),
        "first"
    );
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
    assert_eq!(
        reader
            .query_one("SELECT count(*) FROM records", &[])
            .await
            .unwrap()
            .get::<_, i64>(0),
        i64::from(commit)
    );
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
    assert!(is_unique_violation(&backend(error)));
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
    assert!(
        other
            .query_one("SELECT to_regclass('records')::text", &[])
            .await
            .unwrap()
            .get::<_, Option<String>>(0)
            .is_none()
    );
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
    let options: FindMany<'_, (), ()> = FindMany::default();
    assert_eq!(
        (options.filter, options.order, options.skip, options.take),
        (None, None, None, None)
    );
}
