use super::{failure, input::Cache};
use crate::helpers::HelperResult;
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
    let memory = LocalTtlCache::with_clock(
        Some(1000),
        None,
        None,
        1024,
        Arc::new(move || f64::from(source.load(Ordering::SeqCst))),
    )
    .map_err(failure)?;
    replay(
        input,
        &CredentialCache::new(DualLocalCache {
            memory: Arc::new(Mutex::new(memory)),
            default_in_memory_ttl: None,
            diagnostics: Diagnostics {
                redactor: Arc::new(Redactor::new(16).map_err(failure)?),
                emit: Arc::new(|_| {}),
            },
        })
        .map_err(failure)?,
        &clock,
    )
    .await
}

async fn replay(input: &Cache, storage: &CredentialCache, clock: &AtomicU32) -> HelperResult {
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
                Ok(ResolvedCredentials {
                    credentials: step.access.as_ref().map(|access| Credentials {
                        access_key_id: SecretString::from(access.clone()),
                        secret_access_key: SecretString::from("fixture".to_owned()),
                        session_token: None,
                    }),
                    cache_ttl: step.ttl.map(|ttl| Duration::from_secs(u64::from(ttl))),
                })
            })
            .await;
        results.push(result.map_or_else(
            |_error| json!({"error": true}),
            |value| json!({"access": value.map(|credentials| credentials.access_key_id.expose_secret().to_owned())}),
        ));
    }
    Ok(json!({"fetches": fetches, "results": results}))
}
