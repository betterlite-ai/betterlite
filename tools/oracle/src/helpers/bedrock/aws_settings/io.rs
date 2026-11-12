use super::{failure, input::Cache};
use crate::helpers::{HelperError, HelperResult};
use betterlite_bedrock::aws_settings::{
    credential_cache::CredentialCache,
    sources::{Credentials, ResolvedCredentials, SourceError},
};
use betterlite_context::local_ttl_cache::{
    LocalTtlCache,
    dual::{Diagnostics, DualLocalCache},
};
use betterlite_redact::secret::Redactor;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub(super) async fn cache(input: &Cache) -> HelperResult {
    let clock = Arc::new(AtomicU32::new(0));
    let source = Arc::clone(&clock);
    let memory = Arc::new(Mutex::new(
        LocalTtlCache::with_clock(
            Some(1000),
            None,
            None,
            1024,
            Arc::new(move || f64::from(source.load(Ordering::SeqCst))),
        )
        .map_err(failure)?,
    ));
    let storage = CredentialCache::new(DualLocalCache {
        memory: Arc::clone(&memory),
        default_in_memory_ttl: None,
        diagnostics: Diagnostics {
            redactor: Arc::new(Redactor::new(16).map_err(failure)?),
            emit: Arc::new(|_| {}),
        },
    })
    .map_err(failure)?;
    replay(input, &storage, &clock, &memory).await
}

async fn replay(
    input: &Cache,
    storage: &CredentialCache,
    clock: &AtomicU32,
    memory: &Mutex<LocalTtlCache>,
) -> HelperResult {
    let mut fetches = 0_u32;
    let mut results = Vec::new();
    for step in &input.steps {
        clock.store(step.now, Ordering::SeqCst);
        let result = storage
            .get_or_fetch(&input.parameters, || async {
                fetches = fetches.saturating_add(1);
                if step.fail {
                    return Err(SourceError::MissingProvider);
                }
                let Some(access) = step.access.clone() else {
                    return Err(SourceError::MissingProvider);
                };
                Ok(ResolvedCredentials {
                    credentials: Credentials {
                        access_key_id: SecretString::from(access),
                        secret_access_key: SecretString::from("fixture".to_owned()),
                        session_token: None,
                    },
                    cache_ttl: None,
                })
            })
            .await;
        let ttl = memory_ttl(memory, step.now)?;
        results.push(result.map_or_else(
            |_error| json!({"error": true, "ttl": ttl}),
            |credentials| {
                json!({"access": credentials.access_key_id.expose_secret().to_owned(), "ttl": ttl})
            },
        ));
    }
    Ok(json!({"fetches": fetches, "results": results}))
}

pub(super) fn memory_ttl(
    memory: &Mutex<LocalTtlCache>,
    now: u32,
) -> Result<Option<u64>, Box<HelperError>> {
    let Ok(held) = memory.lock() else {
        return Err(failure("credential cache unavailable"));
    };
    let entry = held
        .ttl_entries()
        .into_iter()
        .map(|(_key, seconds)| seconds)
        .next();
    let Some(expiry) = entry else {
        return Ok(None);
    };
    let expires = Duration::try_from_secs_f64(expiry).map_err(failure)?;
    let elapsed = Duration::from_secs(u64::from(now));
    let remaining = expires
        .checked_sub(elapsed)
        .ok_or_else(|| failure("credential cache expiry is in the past"))?;
    Ok(Some(remaining.as_secs()))
}
