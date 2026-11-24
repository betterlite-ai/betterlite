use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::mcp_sso_assertion_cache::Settings as CacheSettings;
use betterlite_db::sso_assertion_rows::{
    AssertionRow, AssertionRows, MemoryAssertionRows, MemoryFailures, MemoryRowsError,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::credential_contracts::byok_cache::SystemClock;
use betterlite_mcp::sso_assertions::assertion::{SsoIdentityAssertion, decode_payload};
use betterlite_mcp::sso_assertions::cache::{CacheState, SsoAssertionCache};
use betterlite_mcp::sso_assertions::read::{
    AssertionStoreUnavailable, SsoAssertionStore as _, fetch_uncached,
};
use betterlite_mcp::sso_assertions::store::DbSsoAssertionStore;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{Algorithm, CipherSettings, OsNonces, StoredCipher};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::input::{Failure, FailureMethod, Input, Step};
use super::logs::Logs;

const SALT_KEY: &str = "b10064-salt-key";

// Stands in for the login that lands during a fetch's row read and invalidates the entry first.
struct CacheWatchingRows {
    rows: MemoryAssertionRows,
    cache: Arc<SsoAssertionCache>,
    pending: Arc<Mutex<Option<String>>>,
}

impl AssertionRows for CacheWatchingRows {
    type Error = MemoryRowsError;

    async fn find(&self, user_id: &str) -> Result<Option<AssertionRow>, MemoryRowsError> {
        let pending = self
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if pending.as_deref() == Some(user_id) {
            self.cache.invalidate(user_id);
        }
        self.rows.find(user_id).await
    }

    async fn find_all(&self) -> Result<Vec<AssertionRow>, MemoryRowsError> {
        self.rows.find_all().await
    }

    async fn upsert(&self, user_id: &str, assertion_b64: &str) -> Result<(), MemoryRowsError> {
        self.rows.upsert(user_id, assertion_b64).await
    }

    async fn update(&self, user_id: &str, assertion_b64: &str) -> Result<(), MemoryRowsError> {
        self.rows.update(user_id, assertion_b64).await
    }

    async fn id_jag_server_exists(&self) -> Result<bool, MemoryRowsError> {
        self.rows.id_jag_server_exists().await
    }
}

pub(crate) async fn answer(input: &Input, logs: &Logs) -> HelperResult {
    let settings = CacheSettings::default();
    let cache = Arc::new(SsoAssertionCache {
        ttl: Duration::from_secs(settings.ttl_seconds),
        max_size: settings.max_size,
        clock: Box::new(SystemClock),
        state: Mutex::new(CacheState::default()),
    });
    let pending = Arc::new(Mutex::new(None));
    let rows = input.database.then(|| CacheWatchingRows {
        rows: seeded_rows(input),
        cache: Arc::clone(&cache),
        pending: Arc::clone(&pending),
    });
    let store = DbSsoAssertionStore {
        rows,
        cipher: cipher_with(SALT_KEY),
        cache,
        config_declares_id_jag: false,
    };
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(run_step(&store, &pending, step).await?);
    }
    Ok(json!({
        "steps": steps,
        "logs": log_entries(logs),
    }))
}

async fn run_step(
    store: &DbSsoAssertionStore<CacheWatchingRows>,
    pending: &Mutex<Option<String>>,
    step: &Step,
) -> HelperResult {
    match step {
        Step::Fetch {
            user_id,
            invalidate_during_read,
        } => {
            *pending.lock().unwrap_or_else(PoisonError::into_inner) =
                invalidate_during_read.then(|| user_id.clone());
            let outcome = store.fetch(user_id).await;
            *pending.lock().unwrap_or_else(PoisonError::into_inner) = None;
            Ok(report("fetch", outcome, store, user_id))
        }
        Step::FetchUncached { user_id } => Ok(report(
            "fetch_uncached",
            fetch_uncached(store, user_id).await,
            store,
            user_id,
        )),
        Step::Persist { user_id, assertion } => {
            let built = decode_payload(&serde_json::to_string(assertion)?).or_value_error()?;
            store.persist(user_id, &built).await.or_value_error()?;
            Ok(report("persist", Ok(None), store, user_id))
        }
    }
}

fn report(
    step: &str,
    outcome: Result<Option<SsoIdentityAssertion>, AssertionStoreUnavailable>,
    store: &DbSsoAssertionStore<CacheWatchingRows>,
    user_id: &str,
) -> Value {
    let (label, found) = match outcome {
        Ok(found) => ("ok", found),
        Err(_unavailable) => ("unavailable", None),
    };
    json!({
        "step": step,
        "outcome": label,
        "assertion": found.as_ref().map(rendered),
        "cached": store.cache.get(user_id).as_ref().map(rendered),
        "epoch": store.cache.epoch(),
        "reads": reads(store),
        "writes": writes(store),
    })
}

fn rendered(assertion: &SsoIdentityAssertion) -> Value {
    json!({
        "id_token": assertion.id_token.expose_secret(),
        "refresh_token": assertion.refresh_token.as_ref().map(ExposeSecret::expose_secret),
        "issuer": assertion.issuer,
        "expires_at": assertion.expires_at.map(|moment| moment.to_rfc3339()),
    })
}

fn seeded_rows(input: &Input) -> MemoryAssertionRows {
    MemoryAssertionRows {
        rows: Mutex::new(
            input
                .rows
                .iter()
                .map(|seed| AssertionRow {
                    user_id: seed.user_id.clone(),
                    assertion_b64: seed.assertion_b64.clone(),
                })
                .collect(),
        ),
        id_jag: false,
        failures: failures(input.failure.as_ref()),
        reads: AtomicUsize::new(0),
        writes: AtomicUsize::new(0),
    }
}

fn failures(failure: Option<&Failure>) -> MemoryFailures {
    let mut configured = MemoryFailures::default();
    let Some(configured_failure) = failure else {
        return configured;
    };
    match configured_failure.method {
        FailureMethod::Find => configured.find = Some(configured_failure.call),
    }
    configured
}

fn reads(store: &DbSsoAssertionStore<CacheWatchingRows>) -> usize {
    store
        .rows
        .as_ref()
        .map_or(0, |rows| rows.rows.reads.load(Ordering::SeqCst))
}

fn writes(store: &DbSsoAssertionStore<CacheWatchingRows>) -> usize {
    store
        .rows
        .as_ref()
        .map_or(0, |rows| rows.rows.writes.load(Ordering::SeqCst))
}

fn log_entries(logs: &Logs) -> Vec<Value> {
    logs.0
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .into_iter()
        .map(|(level, message)| json!({"level": level, "message": message}))
        .collect()
}

fn cipher_with(key: &str) -> DbCipher {
    StoredCipher {
        settings: CipherSettings {
            signing_key: Some(SecretString::from(key.to_owned())),
            algorithm: Algorithm::XSalsa20Poly1305,
        },
        nonces: OsNonces,
        diagnostics: TracingDiagnostics,
    }
}
