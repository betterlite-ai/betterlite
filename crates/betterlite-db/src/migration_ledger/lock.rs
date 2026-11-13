use std::error::Error as StdError;
use std::future::Future;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use deadpool_postgres::ClientWrapper;
use rand::RngExt as _;
use rand::SeedableRng as _;
use rand::rngs::{StdRng, SysError, SysRng};
use thiserror::Error;
use tokio::time::{Instant, sleep};
use tokio_postgres::{Client, Error as PostgresError, GenericClient};

pub const MIGRATION_LOCK_KEY: i64 = i64::from_be_bytes(*b"llm_mig2");
pub const PRISMA_LOCK_KEY: i64 = 72_707_369;

const XACT_LOCK: &str = "SELECT pg_try_advisory_xact_lock($1) AS acquired";
const SESSION_LOCK: &str = "SELECT pg_try_advisory_lock($1) AS acquired";
const JITTER_FLOOR: Duration = Duration::from_millis(500);
const JITTER_SPAN_NANOS: u64 = 1_000_000_000;

pub trait JitterSource: Send + Sync {
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64;
}

pub struct SystemJitter {
    random: Mutex<StdRng>,
}

impl SystemJitter {
    // The retry pause is drawn from a generator seeded once from the OS, like Python's random module
    pub fn try_new() -> Result<Self, LockError> {
        let random =
            StdRng::try_from_rng(&mut SysRng).map_err(|source| LockError::Entropy { source })?;
        Ok(Self {
            random: Mutex::new(random),
        })
    }
}

impl JitterSource for SystemJitter {
    // The drawn range stays in integer nanoseconds, so no float-to-Duration conversion can panic
    fn uniform_nanos(&self, upper_exclusive_nanos: u64) -> u64 {
        if upper_exclusive_nanos == 0 {
            return 0;
        }
        let mut random = self.random.lock().unwrap_or_else(PoisonError::into_inner);
        random.random_range(0_u64..upper_exclusive_nanos)
    }
}

#[derive(Debug, Error)]
pub enum LockError {
    #[error("lost or could not establish v2 migration coordination with the database")]
    Connection {
        #[source]
        source: PostgresError,
    },
    #[error(
        "Timed out waiting for another v2 migration resolver after {seconds}s. Check the running migration or increase LITELLM_MIGRATION_LOCK_TIMEOUT."
    )]
    CoordinatorTimeout { seconds: u64 },
    #[error(
        "Timed out waiting for Prisma's lock to recover migration history. LiteLLM startup has stopped. Another migration or a pooled database session may still hold the lock. Check the database lock holder. When using a transaction pooler, configure DIRECT_URL to reach the same database without the pooler."
    )]
    PrismaLockTimeout,
    #[error("the migration retry jitter source could not be seeded")]
    Entropy {
        #[source]
        source: SysError,
    },
}

#[derive(Debug, Error)]
pub enum HeldLockError<E> {
    #[error("could not take the migration coordinator session lock")]
    Acquire {
        #[source]
        source: PostgresError,
    },
    #[error(transparent)]
    Body(E),
    #[error("could not release the migration coordinator session lock")]
    Unlock {
        #[source]
        source: PostgresError,
    },
}

// Explicit BEGIN/COMMIT keeps the coordinator's connection private, so no transaction lock can reach a pool.
#[derive(Debug)]
pub struct MigrationCoordinator {
    pub client: ClientWrapper,
    pub lock_timeout: Duration,
}

impl MigrationCoordinator {
    // Mirrors migration_lock.py::migration_lock: a transaction-scoped coordinator lock, retried until the configured deadline
    pub async fn acquire(
        client: ClientWrapper,
        lock_timeout: Duration,
        jitter: &dyn JitterSource,
    ) -> Result<Self, LockError> {
        let deadline = Instant::now().checked_add(lock_timeout);
        tracing::info!(
            wait_seconds = lock_timeout.as_secs(),
            "waiting for the v2 migration coordinator lock"
        );
        loop {
            if deadline.is_some_and(|end| Instant::now() >= end) {
                return Err(LockError::CoordinatorTimeout {
                    seconds: lock_timeout.as_secs(),
                });
            }
            client
                .batch_execute("BEGIN")
                .await
                .map_err(|source| LockError::Connection { source })?;
            if try_lock(&*client, XACT_LOCK, MIGRATION_LOCK_KEY)
                .await
                .map_err(|source| LockError::Connection { source })?
            {
                tracing::info!("acquired the v2 migration coordinator lock");
                return Ok(Self {
                    client,
                    lock_timeout,
                });
            }
            client
                .batch_execute("ROLLBACK")
                .await
                .map_err(|source| LockError::Connection { source })?;
            sleep(pause(jitter, deadline)).await;
        }
    }

    // Mirrors MigrationCoordinator.acquire_prisma_lock: Prisma's own transaction lock on the held transaction
    pub async fn acquire_prisma_lock(&self, jitter: &dyn JitterSource) -> Result<(), LockError> {
        let acquired = wait_for_lock(&*self.client, PRISMA_LOCK_KEY, self.lock_timeout, jitter)
            .await
            .map_err(|source| LockError::Connection { source })?;
        if acquired {
            return Ok(());
        }
        Err(LockError::PrismaLockTimeout)
    }

    pub async fn check_connection(&self) -> Result<(), PostgresError> {
        self.client
            .execute("SELECT 1", &[])
            .await
            .map(|_affected| ())
    }

    // The caller's work is durable only through this checkpoint, which proves the transaction is still usable first
    pub async fn finish(self) -> Result<(), LockError> {
        self.check_connection()
            .await
            .map_err(|source| LockError::Connection { source })?;
        self.client
            .batch_execute("COMMIT")
            .await
            .map_err(|source| LockError::Connection { source })
    }
}

// Mirrors held_migration_lock: a session-level, non-blocking hold for DDL that cannot run inside a transaction
pub async fn held_migration_lock<T, E, F, Fut>(
    client: &Client,
    body: F,
) -> Result<T, HeldLockError<E>>
where
    F: FnOnce(bool) -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: StdError + 'static,
{
    let acquired = try_lock(client, SESSION_LOCK, MIGRATION_LOCK_KEY)
        .await
        .map_err(|source| HeldLockError::Acquire { source })?;
    let outcome = body(acquired).await;
    let released = if acquired {
        client
            .execute("SELECT pg_advisory_unlock($1)", &[&MIGRATION_LOCK_KEY])
            .await
            .map(|_affected| ())
    } else {
        Ok(())
    };
    match (outcome, released) {
        (_, Err(source)) => Err(HeldLockError::Unlock { source }),
        (Err(source), Ok(())) => Err(HeldLockError::Body(source)),
        (Ok(value), Ok(())) => Ok(value),
    }
}

// One advisory-lock probe; the statement decides between the transaction and session variants
async fn try_lock<C>(client: &C, statement: &str, key: i64) -> Result<bool, PostgresError>
where
    C: GenericClient + Sync,
{
    client
        .query_opt(statement, &[&key])
        .await?
        .map_or(Ok(false), |found| found.try_get(0))
}

// Both coordinator locks retry a non-blocking probe with the same jittered pause until their deadline
async fn wait_for_lock<C>(
    client: &C,
    key: i64,
    timeout: Duration,
    jitter: &dyn JitterSource,
) -> Result<bool, PostgresError>
where
    C: GenericClient + Sync,
{
    let deadline = Instant::now().checked_add(timeout);
    loop {
        if deadline.is_some_and(|end| Instant::now() >= end) {
            return Ok(false);
        }
        if try_lock(client, XACT_LOCK, key).await? {
            return Ok(true);
        }
        sleep(pause(jitter, deadline)).await;
    }
}

pub(super) fn pause(jitter: &dyn JitterSource, deadline: Option<Instant>) -> Duration {
    let drawn = JITTER_FLOOR.saturating_add(Duration::from_nanos(
        jitter.uniform_nanos(JITTER_SPAN_NANOS),
    ));
    deadline.map_or(drawn, |end| {
        drawn.min(end.saturating_duration_since(Instant::now()))
    })
}
