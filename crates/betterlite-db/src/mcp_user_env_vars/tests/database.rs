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
use crate::mcp_user_env_vars::postgres::postgres_user_env_vars_store;
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

fn cipher() -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(SALT.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}

async fn seeded() -> (PostgresFixture, PostgresStore) {
    let configured = Environment::parse_from(["mcp-user-env-vars-database"])
        .url
        .map(SecretString::from);
    let fixture = PostgresFixture::provision_schema(configured.as_ref())
        .resolve()
        .await
        .expect("the fixture provisions a schema");
    let config: Config = fixture.options.url.expose_secret().parse().unwrap();
    let pool = Pool::builder(Manager::new(config, NoTls))
        .max_size(4)
        .build()
        .unwrap();
    pool.get()
        .await
        .unwrap()
        .batch_execute(STORAGE)
        .await
        .expect("the test table exists");
    (fixture, PostgresStore { pool: Some(pool) })
}

async fn seed(
    db: &PostgresStore,
    cipher: &DbCipher,
    user_id: &str,
    server_id: &str,
    values: UserEnvVars,
) {
    let sealed = seal_user_env_vars(&values, cipher).expect("the test values seal");
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
    let cipher = cipher();
    seed(
        &db,
        &cipher,
        "alice",
        "srv-1",
        secrets([("API_KEY", "s3cr3t")]),
    )
    .await;
    seed(
        &db,
        &cipher,
        "alice",
        "srv-2",
        secrets([("TOKEN", "tok-2")]),
    )
    .await;
    seed(&db, &cipher, "bob", "srv-1", secrets([("API_KEY", "bobs")])).await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &db,
        reader: &db,
        availability: &availability,
    };
    let store = postgres_user_env_vars_store(Some(client), publisher(), cipher())
        .expect("the postgres store builds");
    let found = store
        .get_user_env_vars("alice", "srv-1")
        .await
        .expect("the seeded row decodes");
    assert_eq!(exposed(&found), texts([("API_KEY", "s3cr3t")]));
    let missing = store
        .get_user_env_vars("alice", "srv-3")
        .await
        .expect("a missing row is an empty map");
    assert!(missing.is_empty());
    let ids = ["srv-1".to_owned(), "srv-2".to_owned(), "srv-3".to_owned()];
    let bulk = store
        .get_user_env_vars_bulk("alice", &ids)
        .await
        .expect("the seeded rows decode");
    assert_eq!(bulk.len(), 2);
    assert_eq!(exposed(&bulk["srv-1"]), texts([("API_KEY", "s3cr3t")]));
    assert_eq!(exposed(&bulk["srv-2"]), texts([("TOKEN", "tok-2")]));
    assert!(!bulk.contains_key("srv-3"));
}

#[tokio::test]
async fn merge_creates_and_overwrites_one_sealed_row_in_a_transaction() {
    let (_fixture, db) = seeded().await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &db,
        reader: &db,
        availability: &availability,
    };
    let store = postgres_user_env_vars_store(Some(client), publisher(), cipher())
        .expect("the postgres store builds");
    let created = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &secrets([("API_KEY", "fresh"), ("EXTRA", "x")]),
            &names(&["API_KEY", "EXTRA"]),
        )
        .await
        .expect("the merge commits");
    assert_eq!(
        exposed(&created),
        texts([("API_KEY", "fresh"), ("EXTRA", "x")])
    );
    let pruned = store
        .merge_user_env_vars(
            "alice",
            "srv-1",
            &secrets([("API_KEY", "rotated")]),
            &names(&["API_KEY"]),
        )
        .await
        .expect("the second merge commits");
    assert_eq!(exposed(&pruned), texts([("API_KEY", "rotated")]));
    let stored = store
        .get_user_env_vars("alice", "srv-1")
        .await
        .expect("the merged row reads back");
    assert_eq!(exposed(&stored), texts([("API_KEY", "rotated")]));
}

#[tokio::test]
async fn delete_removes_only_the_named_row() {
    let (_fixture, db) = seeded().await;
    let cipher = cipher();
    seed(&db, &cipher, "alice", "srv-1", secrets([("API_KEY", "a")])).await;
    seed(&db, &cipher, "alice", "srv-2", secrets([("TOKEN", "b")])).await;
    seed(&db, &cipher, "bob", "srv-1", secrets([("API_KEY", "c")])).await;
    let availability = Availability::default();
    let client = PrismaClient {
        writer: &db,
        reader: &db,
        availability: &availability,
    };
    let store = postgres_user_env_vars_store(Some(client), publisher(), cipher())
        .expect("the postgres store builds");
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
    assert_eq!(exposed(&remaining["srv-2"]), texts([("TOKEN", "b")]));
    let bobs = store
        .get_user_env_vars("bob", "srv-1")
        .await
        .expect("the other user's row reads");
    assert_eq!(exposed(&bobs), texts([("API_KEY", "c")]));
    store
        .delete_user_env_vars("alice", "srv-1")
        .await
        .expect("a missing row is a no-op");
}
