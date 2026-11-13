use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use betterlite_context::local_ttl_cache::CacheValue;
use rusqlite::OptionalExtension as _;
use rusqlite::{Connection, TransactionBehavior, params};

use super::{DiskError, LOCK_TIMEOUT};

const CACHE_FILE_NAME: &str = "cache.db";

const CREATE_TABLE: &str =
    "CREATE TABLE IF NOT EXISTS cache (key TEXT PRIMARY KEY, body TEXT NOT NULL, expire_at REAL)";

const UPSERT: &str = "INSERT INTO cache (key, body, expire_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET body = excluded.body, expire_at = excluded.expire_at";

const SELECT: &str = "SELECT body, expire_at FROM cache WHERE key = ?1";

const DELETE_UNEXPIRED: &str =
    "DELETE FROM cache WHERE key = ?1 AND (expire_at IS NULL OR expire_at > ?2)";

const CLEAR: &str = "DELETE FROM cache";

pub(super) struct DiskStore {
    connection: Mutex<Connection>,
}

impl DiskStore {
    pub(super) fn open(directory: &Path) -> Result<Self, DiskError> {
        fs::create_dir_all(directory).map_err(|source| DiskError::Directory {
            path: directory.to_owned(),
            source,
        })?;
        let connection = Connection::open(directory.join(CACHE_FILE_NAME))
            .map_err(|source| DiskError::Store { source })?;
        connection
            .busy_timeout(LOCK_TIMEOUT)
            .map_err(|source| DiskError::Store { source })?;
        connection
            .execute(CREATE_TABLE, [])
            .map_err(|source| DiskError::Store { source })?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn connect(&self) -> Result<MutexGuard<'_, Connection>, DiskError> {
        self.connection
            .lock()
            .map_err(|_poisoned| DiskError::Unavailable)
    }

    pub(super) fn set(
        &self,
        key: &str,
        value: &CacheValue,
        expire_at: Option<f64>,
    ) -> Result<(), DiskError> {
        let body = serde_json::to_string(value).map_err(|source| DiskError::Serialize {
            key: key.to_owned(),
            source,
        })?;
        let connection = self.connect()?;
        connection
            .execute(UPSERT, params![key, body, expire_at])
            .map_err(|source| DiskError::Store { source })?;
        drop(connection);
        Ok(())
    }

    pub(super) fn get(&self, key: &str, now: f64) -> Result<Option<CacheValue>, DiskError> {
        let connection = self.connect()?;
        let stored = read(&connection, key, now);
        drop(connection);
        stored
    }

    // LiteLLM pops the key, and diskcache's pop returns None for a key that is absent or expired
    pub(super) fn delete(&self, key: &str, now: f64) -> Result<usize, DiskError> {
        let connection = self.connect()?;
        connection
            .execute(DELETE_UNEXPIRED, params![key, now])
            .map_err(|source| DiskError::Store { source })
    }

    pub(super) fn clear(&self) -> Result<usize, DiskError> {
        let connection = self.connect()?;
        connection
            .execute(CLEAR, [])
            .map_err(|source| DiskError::Store { source })
    }

    pub(super) fn increment(
        &self,
        key: &str,
        value: i64,
        now: f64,
        expire_at: Option<f64>,
    ) -> Result<i64, DiskError> {
        let mut connection = self.connect()?;
        // diskcache's transact() opens BEGIN IMMEDIATE, so the write lock is taken before the read
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|source| DiskError::Store { source })?;
        // LiteLLM starts from the stored int and from zero for anything else, so the read and the write share one transaction
        let base = match read(&transaction, key, now)? {
            Some(CacheValue::Integer(number)) => number,
            Some(_) | None => 0,
        };
        let next = base
            .checked_add(value)
            .ok_or_else(|| DiskError::IncrementOutOfRange {
                key: key.to_owned(),
            })?;
        let body = serde_json::to_string(&CacheValue::Integer(next)).map_err(|source| {
            DiskError::Serialize {
                key: key.to_owned(),
                source,
            }
        })?;
        transaction
            .execute(UPSERT, params![key, body, expire_at])
            .map_err(|source| DiskError::Store { source })?;
        transaction
            .commit()
            .map_err(|source| DiskError::Store { source })?;
        drop(connection);
        Ok(next)
    }
}

fn read(connection: &Connection, key: &str, now: f64) -> Result<Option<CacheValue>, DiskError> {
    let stored = connection
        .query_row(SELECT, params![key], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<f64>>(1)?))
        })
        .optional()
        .map_err(|source| DiskError::Store { source })?;
    let Some((body, expire_at)) = stored else {
        return Ok(None);
    };
    if expire_at.is_some_and(|expiry| expiry <= now) {
        return Ok(None);
    }
    let parsed = serde_json::from_str(&body).map_err(|source| DiskError::Decode {
        key: key.to_owned(),
        source,
    })?;
    serde_json::from_value(parsed)
        .map(Some)
        .map_err(|source| DiskError::Decode {
            key: key.to_owned(),
            source,
        })
}
