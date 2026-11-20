use std::error::Error as StdError;
use std::future::{Future, ready};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use betterlite_secrets::secret_codec::{DecryptPolicy, FailureLog, FailureValue};
use thiserror::Error;
use tokio_postgres::Row;

use crate::postgres_store::errors::{DbError, backend};
use crate::postgres_store::session::PostgresStore;

// LiteLLM's _ASSERTION_DECRYPT_LOG_KEY: the read and the rotation name a failed row by this field.
pub const ASSERTION_DECRYPT_FIELD: &str = "sso_identity_assertion";
// decrypt_value_helper's exception_type="debug": an unreadable row reads as absent, never as its text.
pub const ASSERTION_DECRYPT_POLICY: DecryptPolicy = DecryptPolicy {
    log: FailureLog::Debug,
    value: FailureValue::Nothing,
};
pub const ID_JAG_AUTH_TYPE: &str = "oauth2_id_jag";

const FIND_SQL: &str =
    r#"SELECT user_id, assertion_b64 FROM "LiteLLM_SSOIdentityAssertion" WHERE user_id = $1"#;
// ordered, so two rotations of the same table warn in the same order
const FIND_ALL_SQL: &str =
    r#"SELECT user_id, assertion_b64 FROM "LiteLLM_SSOIdentityAssertion" ORDER BY user_id"#;
// the row has no trigger: the DO UPDATE arm stamps updated_at the way Prisma's @updatedAt does
const UPSERT_SQL: &str = r#"INSERT INTO "LiteLLM_SSOIdentityAssertion" (user_id, assertion_b64)
    VALUES ($1, $2)
    ON CONFLICT (user_id) DO UPDATE
    SET assertion_b64 = EXCLUDED.assertion_b64, updated_at = CURRENT_TIMESTAMP"#;
const UPDATE_SQL: &str = r#"UPDATE "LiteLLM_SSOIdentityAssertion"
    SET assertion_b64 = $2, updated_at = CURRENT_TIMESTAMP
    WHERE user_id = $1"#;
// _MCPServerTable.find_first: auth_type alone decides, so a pending or disabled server still counts
const ID_JAG_SQL: &str = r#"SELECT 1 FROM "LiteLLM_MCPServerTable" WHERE auth_type = $1 LIMIT 1"#;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssertionRow {
    pub user_id: String,
    pub assertion_b64: String,
}

// The rows of _SSOAssertionTable and the one lookup of _MCPServerTable the store calls.
pub trait AssertionRows {
    type Error: StdError + Send + Sync + 'static;

    fn find(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<AssertionRow>, Self::Error>> + Send;

    fn find_all(&self) -> impl Future<Output = Result<Vec<AssertionRow>, Self::Error>> + Send;

    fn upsert(
        &self,
        user_id: &str,
        assertion_b64: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn update(
        &self,
        user_id: &str,
        assertion_b64: &str,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    fn id_jag_server_exists(&self) -> impl Future<Output = Result<bool, Self::Error>> + Send;
}

pub struct PostgresAssertionRows {
    pub store: PostgresStore,
}

fn decode_assertion(row: &Row) -> Result<AssertionRow, DbError> {
    Ok(AssertionRow {
        user_id: row.try_get("user_id").map_err(backend)?,
        assertion_b64: row.try_get("assertion_b64").map_err(backend)?,
    })
}

impl AssertionRows for PostgresAssertionRows {
    type Error = DbError;

    async fn find(&self, user_id: &str) -> Result<Option<AssertionRow>, DbError> {
        let client = self.store.session().await?;
        let row = client
            .query_opt(FIND_SQL, &[&user_id])
            .await
            .map_err(backend)?;
        row.as_ref().map(decode_assertion).transpose()
    }

    async fn find_all(&self) -> Result<Vec<AssertionRow>, DbError> {
        let client = self.store.session().await?;
        let rows = client.query(FIND_ALL_SQL, &[]).await.map_err(backend)?;
        rows.iter().map(decode_assertion).collect()
    }

    async fn upsert(&self, user_id: &str, assertion_b64: &str) -> Result<(), DbError> {
        let client = self.store.session().await?;
        client
            .execute(UPSERT_SQL, &[&user_id, &assertion_b64])
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn update(&self, user_id: &str, assertion_b64: &str) -> Result<(), DbError> {
        let client = self.store.session().await?;
        // no row matched is not an error: the row was already gone when the rotation reached it
        client
            .execute(UPDATE_SQL, &[&user_id, &assertion_b64])
            .await
            .map_err(backend)?;
        Ok(())
    }

    async fn id_jag_server_exists(&self) -> Result<bool, DbError> {
        let client = self.store.session().await?;
        let row = client
            .query_opt(ID_JAG_SQL, &[&ID_JAG_AUTH_TYPE])
            .await
            .map_err(backend)?;
        Ok(row.is_some())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssertionRowsMethod {
    Find,
    FindAll,
    Upsert,
    Update,
    IdJagServerExists,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("the in-memory assertion rows failed on {method:?}")]
pub struct MemoryRowsError {
    pub method: AssertionRowsMethod,
}

// Each field fails its method once that many reads or writes have been accepted; None never fails.
#[derive(Debug, Clone, Copy, Default)]
pub struct MemoryFailures {
    pub find: Option<usize>,
    pub find_all: Option<usize>,
    pub upsert: Option<usize>,
    pub update: Option<usize>,
    pub id_jag_server_exists: Option<usize>,
}

// The shared fake: insertion order, the id_jag flag, call counters and a per-method failure.
#[derive(Debug, Default)]
pub struct MemoryAssertionRows {
    pub rows: Mutex<Vec<AssertionRow>>,
    pub id_jag: bool,
    pub failures: MemoryFailures,
    pub reads: AtomicUsize,
    pub writes: AtomicUsize,
}

impl AssertionRows for MemoryAssertionRows {
    type Error = MemoryRowsError;

    fn find(
        &self,
        user_id: &str,
    ) -> impl Future<Output = Result<Option<AssertionRow>, MemoryRowsError>> + Send {
        if self.failures.find == Some(self.reads.load(Ordering::SeqCst)) {
            return ready(Err(MemoryRowsError {
                method: AssertionRowsMethod::Find,
            }));
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let found = rows.iter().find(|row| row.user_id == user_id).cloned();
        ready(Ok(found))
    }

    fn find_all(&self) -> impl Future<Output = Result<Vec<AssertionRow>, MemoryRowsError>> + Send {
        if self.failures.find_all == Some(self.reads.load(Ordering::SeqCst)) {
            return ready(Err(MemoryRowsError {
                method: AssertionRowsMethod::FindAll,
            }));
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        let rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        ready(Ok(rows.clone()))
    }

    fn upsert(
        &self,
        user_id: &str,
        assertion_b64: &str,
    ) -> impl Future<Output = Result<(), MemoryRowsError>> + Send {
        if self.failures.upsert == Some(self.writes.load(Ordering::SeqCst)) {
            return ready(Err(MemoryRowsError {
                method: AssertionRowsMethod::Upsert,
            }));
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        let mut replaced = false;
        for row in rows.iter_mut() {
            if row.user_id == user_id {
                row.assertion_b64 = assertion_b64.to_owned();
                replaced = true;
                break;
            }
        }
        if !replaced {
            rows.push(AssertionRow {
                user_id: user_id.to_owned(),
                assertion_b64: assertion_b64.to_owned(),
            });
        }
        ready(Ok(()))
    }

    fn update(
        &self,
        user_id: &str,
        assertion_b64: &str,
    ) -> impl Future<Output = Result<(), MemoryRowsError>> + Send {
        if self.failures.update == Some(self.writes.load(Ordering::SeqCst)) {
            return ready(Err(MemoryRowsError {
                method: AssertionRowsMethod::Update,
            }));
        }
        self.writes.fetch_add(1, Ordering::SeqCst);
        let mut rows = self.rows.lock().unwrap_or_else(PoisonError::into_inner);
        for row in rows.iter_mut() {
            if row.user_id == user_id {
                row.assertion_b64 = assertion_b64.to_owned();
            }
        }
        ready(Ok(()))
    }

    fn id_jag_server_exists(&self) -> impl Future<Output = Result<bool, MemoryRowsError>> + Send {
        if self.failures.id_jag_server_exists == Some(self.reads.load(Ordering::SeqCst)) {
            return ready(Err(MemoryRowsError {
                method: AssertionRowsMethod::IdJagServerExists,
            }));
        }
        self.reads.fetch_add(1, Ordering::SeqCst);
        ready(Ok(self.id_jag))
    }
}

mod tests;
