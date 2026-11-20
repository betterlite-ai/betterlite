use std::convert::Infallible;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::mcp_sso_assertion_cache::Settings as CacheSettings;
use betterlite_db::sso_assertion_rows::{
    ASSERTION_DECRYPT_FIELD, ASSERTION_DECRYPT_POLICY, AssertionRow, MemoryAssertionRows,
    MemoryFailures,
};
use betterlite_db::stored_cipher::DbCipher;
use betterlite_mcp::credential_contracts::byok_cache::SystemClock;
use betterlite_mcp::sso_assertions::assertion::{SsoIdentityAssertion, decode_payload};
use betterlite_mcp::sso_assertions::cache::{CacheState, SsoAssertionCache};
use betterlite_mcp::sso_assertions::store::DbSsoAssertionStore;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::HelperResult;
use betterlite_secrets::secret_codec::diagnostics::TracingDiagnostics;
use betterlite_secrets::secret_codec::{
    Algorithm, CipherSettings, OsNonces, StoredCipher, StoredValue,
};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};

use super::input::{Failure, FailureMethod, Input, Step};
use super::logs::Logs;

const SALT_KEY: &str = "b10011-salt-key";
const ID_JAG: &str = "oauth2_id_jag";

pub(crate) async fn answer(input: &Input, logs: &Logs) -> HelperResult {
    let settings = CacheSettings::default();
    let cache = Arc::new(SsoAssertionCache {
        ttl: Duration::from_secs(settings.ttl_seconds),
        max_size: settings.max_size,
        clock: Box::new(SystemClock),
        state: Mutex::new(CacheState::default()),
    });
    let rows = input.database.then(|| memory_rows(input));
    let store = DbSsoAssertionStore {
        rows,
        cipher: cipher_with(SALT_KEY),
        cache: Arc::clone(&cache),
        config_declares_id_jag: input
            .config_servers
            .iter()
            .flatten()
            .any(|auth| auth == ID_JAG),
    };
    let mut steps = Vec::with_capacity(input.steps.len());
    for step in &input.steps {
        steps.push(run_step(&store, &cache, step).await?);
    }
    Ok(json!({
        "steps": steps,
        "stored": stored_rows(store.rows.as_ref(), &store.cipher),
        "reads": reads(&store),
        "writes": writes(&store),
        "logs": log_entries(logs),
    }))
}

async fn run_step(
    store: &DbSsoAssertionStore<MemoryAssertionRows>,
    cache: &SsoAssertionCache,
    step: &Step,
) -> HelperResult {
    match step {
        Step::CacheSet {
            user_id,
            assertion,
            seen_epoch,
        } => {
            let built = decode_payload(&serde_json::to_string(assertion)?).or_value_error()?;
            cache.set_if_unchanged(user_id, &built, *seen_epoch);
            Ok(json!({"step": "cache_set", "epoch": cache.epoch()}))
        }
        Step::CacheGet { user_id } => Ok(json!({
            "step": "cache_get",
            "epoch": cache.epoch(),
            "assertion": cache.get(user_id).as_ref().map(rendered),
        })),
        Step::Invalidate { user_id } => {
            cache.invalidate(user_id);
            Ok(json!({"step": "invalidate", "epoch": cache.epoch()}))
        }
        Step::Persist { user_id, assertion } => {
            let built = decode_payload(&serde_json::to_string(assertion)?).or_value_error()?;
            store.persist(user_id, &built).await.or_value_error()?;
            Ok(json!({"step": "persist", "epoch": cache.epoch()}))
        }
        Step::Retention => Ok(json!({
            "step": "retention",
            "enabled": store.retention_enabled().await.or_value_error()?,
        })),
        Step::Retain { user_id, assertion } => {
            let built = assertion
                .as_deref()
                .map(|seed| decode_payload(&serde_json::to_string(seed)?).or_value_error())
                .transpose()?;
            store.retain(user_id, built.as_ref()).await;
            Ok(json!({"step": "retain"}))
        }
    }
}

fn rendered(assertion: &SsoIdentityAssertion) -> Value {
    json!({
        "id_token": assertion.id_token.expose_secret(),
        "refresh_token": assertion.refresh_token.as_ref().map(ExposeSecret::expose_secret),
        "issuer": assertion.issuer,
        "expires_at": assertion.expires_at.map(|moment| moment.to_rfc3339()),
    })
}

fn memory_rows(input: &Input) -> MemoryAssertionRows {
    MemoryAssertionRows {
        rows: Mutex::new(Vec::new()),
        id_jag: input.db_servers.iter().flatten().any(|auth| auth == ID_JAG),
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
        FailureMethod::Upsert => configured.upsert = Some(configured_failure.call),
        FailureMethod::FindFirst => configured.id_jag_server_exists = Some(configured_failure.call),
    }
    configured
}

fn stored_rows(rows: Option<&MemoryAssertionRows>, cipher: &DbCipher) -> Vec<Value> {
    let Some(present) = rows else {
        return Vec::new();
    };
    let locked = present.rows.lock().unwrap_or_else(PoisonError::into_inner);
    locked.iter().map(|row| stored_row(row, cipher)).collect()
}

fn stored_row(row: &AssertionRow, cipher: &DbCipher) -> Value {
    let sealed = StoredValue::<Infallible>::Text(SecretString::from(row.assertion_b64.clone()));
    let opened = cipher.decrypt_value(sealed, ASSERTION_DECRYPT_FIELD, ASSERTION_DECRYPT_POLICY);
    let assertion = match opened {
        Some(StoredValue::Text(plaintext)) => decode_payload(plaintext.expose_secret()).ok(),
        Some(StoredValue::Other(never)) => match never {},
        None => None,
    };
    json!({
        "user_id": row.user_id,
        "assertion": assertion.as_ref().map(rendered),
    })
}

fn reads(store: &DbSsoAssertionStore<MemoryAssertionRows>) -> usize {
    store
        .rows
        .as_ref()
        .map_or(0, |rows| rows.reads.load(Ordering::SeqCst))
}

fn writes(store: &DbSsoAssertionStore<MemoryAssertionRows>) -> usize {
    store
        .rows
        .as_ref()
        .map_or(0, |rows| rows.writes.load(Ordering::SeqCst))
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
