use std::sync::Arc;

use betterlite_postgres_fixture::connection::PostgresFixture;
use betterlite_redis::config_sync::publish::{PublishChange, PublishFuture};
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use clap::Parser;
use deadpool_postgres::{GenericClient as _, Manager, Pool};
use secrecy::{ExposeSecret as _, SecretString};
use tokio_postgres::{Config, NoTls};
use uuid::Uuid;

use crate::auxiliary_repository::repository::{PrismaClient, TableError};
use crate::database_replica::availability::Availability;
use crate::mcp_user_env_vars::codec::seal_user_env_vars;
use crate::mcp_user_env_vars::postgres::{PostgresUserEnvVarsStore, postgres_user_env_vars_store};
use crate::mcp_user_env_vars::store::UserEnvVars;
use crate::postgres_store::session::PostgresStore;
use crate::stored_cipher::DbCipher;

use super::fakes::SALT;
use super::{exposed, names, secrets, texts};

const STORAGE: &str = r#"
CREATE TABLE "LiteLLM_MCPUserEnvVars" (
    id text PRIMARY KEY,
    user_id text NOT NULL,
    server_id text NOT NULL,
    values_b64 text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (user_id, server_id)
);
"#;

#[derive(Parser)]
struct Environment {
    #[arg(long, env = "POSTGRES_TEST_URL")]
    url: Option<String>,
}

fn publisher() -> PublishChange<TableError> {
    Arc::new(|_name: &str| -> PublishFuture<TableError> { Box::pin(async { Ok(()) }) })
}

fn db_cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

fn store_for<'db>(
    db: &'db PostgresStore,
    availability: &'db Availability,
) -> PostgresUserEnvVarsStore<'db> {
    let client = PrismaClient {
        writer: db,
        reader: db,
        availability,
    };
    postgres_user_env_vars_store(Some(client), publisher(), db_cipher())
        .expect("the postgres store builds")
}

async fn seeded() -> (PostgresFixture, PostgresStore) {
    let configured = Environment::parse_from(["mcp-user-env-vars-database"])
        .url
        .map(SecretString::from);
    let fixture =
        PostgresFixture::provision_with_statements(configured.as_ref(), vec![STORAGE.to_owned()])
            .resolve()
            .await
            .expect("the fixture provisions a database");
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .unwrap();
    (fixture, PostgresStore { pool: Some(pool) })
}

async fn seed(
    db: &PostgresStore,
    cipher: &DbCipher,
    user_id: &str,
    server_id: &str,
    values: &UserEnvVars,
) {
    let sealed = seal_user_env_vars(values, cipher).expect("the test values seal");
    let session = db.session().await.expect("the fixture connects");
    session
        .execute(
            "INSERT INTO \"LiteLLM_MCPUserEnvVars\" (id, user_id, server_id, values_b64) VALUES ($1, $2, $3, $4)",
            &[&Uuid::new_v4().to_string(), &user_id, &server_id, &sealed],
        )
        .await
        .expect("the test row inserts");
}

#[tokio::test]
async fn reads_one_users_rows_through_the_table_handle() {
    let (_fixture, db) = seeded().await;
    let cipher = db_cipher();
    seed(&db, &cipher, "alice", "srv-1", &secrets([("K", "one")])).await;
    let availability = Availability::default();
    let store = store_for(&db, &availability);
    let found = store
        .get_user_env_vars("alice", "srv-1")
        .await
        .expect("the seeded row decodes");
    assert_eq!(exposed(&found), texts([("K", "one")]));
    let missing = store
        .get_user_env_vars("alice", "srv-3")
        .await
        .expect("a missing row is an empty map");
    assert!(missing.is_empty());
}

#[tokio::test]
async fn bulk_reads_only_the_requested_servers() {
    let (_fixture, db) = seeded().await;
    let cipher = db_cipher();
    seed(&db, &cipher, "alice", "srv-1", &secrets([("K", "one")])).await;
    seed(&db, &cipher, "alice", "srv-2", &secrets([("K", "two")])).await;
    seed(&db, &cipher, "bob", "srv-1", &secrets([("K", "bob")])).await;
    let availability = Availability::default();
    let store = store_for(&db, &availability);
    let ids = ["srv-1".to_owned(), "srv-2".to_owned(), "srv-3".to_owned()];
    let bulk = store
        .get_user_env_vars_bulk("alice", &ids)
        .await
        .expect("the seeded rows decode");
    assert_eq!(bulk.len(), 2);
    assert_eq!(exposed(&bulk["srv-1"]), texts([("K", "one")]));
    assert_eq!(exposed(&bulk["srv-2"]), texts([("K", "two")]));
    assert!(!bulk.contains_key("srv-3"));
}

#[tokio::test]
async fn merge_creates_and_overwrites_one_sealed_row_in_a_transaction() {
    let (_fixture, db) = seeded().await;
    let availability = Availability::default();
    let store = store_for(&db, &availability);
    let created = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &secrets([("K", "one"), ("EXTRA", "x")]),
            &names(&["K", "EXTRA"]),
        )
        .await
        .expect("the merge commits");
    assert_eq!(exposed(&created), texts([("EXTRA", "x"), ("K", "one")]));
    let kept = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &secrets([("K", "two")]),
            &names(&["K", "EXTRA"]),
        )
        .await
        .expect("the second merge commits");
    assert_eq!(exposed(&kept), texts([("EXTRA", "x"), ("K", "two")]));
    let reread = store
        .get_user_env_vars("alice", "srv-1")
        .await
        .expect("the merged row reads back");
    assert_eq!(exposed(&reread), texts([("EXTRA", "x"), ("K", "two")]));
}

#[tokio::test]
async fn merge_prunes_names_outside_the_allowed_set() {
    let (_fixture, db) = seeded().await;
    let availability = Availability::default();
    let store = store_for(&db, &availability);
    let created = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &secrets([("K", "one"), ("EXTRA", "x")]),
            &names(&["K", "EXTRA"]),
        )
        .await
        .expect("the first merge commits");
    assert_eq!(exposed(&created), texts([("EXTRA", "x"), ("K", "one")]));
    let pruned = store
        .merge_user_env_vars("alice", "srv-1", &secrets([("K", "two")]), &names(&["K"]))
        .await
        .expect("the second merge commits");
    assert_eq!(exposed(&pruned), texts([("K", "two")]));
    let reread = store
        .get_user_env_vars("alice", "srv-1")
        .await
        .expect("the pruned row reads back");
    assert_eq!(exposed(&reread), texts([("K", "two")]));
}

#[tokio::test]
async fn delete_removes_only_the_named_row() {
    let (_fixture, db) = seeded().await;
    let cipher = db_cipher();
    seed(&db, &cipher, "alice", "srv-1", &secrets([("K", "one")])).await;
    seed(&db, &cipher, "alice", "srv-2", &secrets([("K", "two")])).await;
    seed(&db, &cipher, "bob", "srv-1", &secrets([("K", "bob")])).await;
    let availability = Availability::default();
    let store = store_for(&db, &availability);
    store
        .delete_user_env_vars("alice", "srv-1")
        .await
        .expect("the delete succeeds");
    let ids = ["srv-1".to_owned(), "srv-2".to_owned()];
    let remaining = store
        .get_user_env_vars_bulk("alice", &ids)
        .await
        .expect("the rows read");
    assert!(!remaining.contains_key("srv-1"));
    assert_eq!(exposed(&remaining["srv-2"]), texts([("K", "two")]));
    let bobs = store
        .get_user_env_vars("bob", "srv-1")
        .await
        .expect("the other row reads");
    assert_eq!(exposed(&bobs), texts([("K", "bob")]));
    store
        .delete_user_env_vars("alice", "srv-1")
        .await
        .expect("a missing row is a no-op");
}
