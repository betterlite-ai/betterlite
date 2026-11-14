use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::future::Future;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use sha2::{Digest as _, Sha256};

use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::session::session_id_from_request;

mod tests;

// LiteLLM's DualCache, as far as the scan cache uses it
pub trait ScanCacheStore: Sync {
    type Error: Error + Send + Sync + 'static;

    fn get(
        &self,
        key: &str,
    ) -> impl Future<Output = Result<Option<Vec<String>>, Self::Error>> + Send;

    fn set(
        &self,
        key: &str,
        hashes: Vec<String>,
        ttl_seconds: i64,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;
}

// LiteLLM's _scanned_texts_cache_key prints a missing guardrail name as None
#[must_use]
pub fn scan_cache_key(guardrail_name: Option<&str>, session_id: &str) -> String {
    format!(
        "guardrail_scanned_texts:{}:{session_id}",
        guardrail_name.unwrap_or("None")
    )
}

// LiteLLM's _scanned_text_hash for each text: the lowercase hex SHA-256 of the UTF-8 bytes
fn scanned_text_hashes(texts: &[String]) -> Vec<String> {
    texts
        .iter()
        .map(|text| hex::encode(Sha256::digest(text.as_bytes())))
        .collect()
}

impl GuardrailBase {
    // None means "scan everything"; a list, even an empty one, means "scan only these"
    pub async fn filter_new_texts_for_session<S: ScanCacheStore>(
        &self,
        texts: Option<&[String]>,
        request_data: &BTreeMap<String, GuardrailValue>,
        cache: &S,
    ) -> Option<Vec<String>> {
        let settings = &self.settings;
        let present = texts.filter(|list| !list.is_empty())?;
        if !settings.only_scan_new_messages {
            return None;
        }
        let name = settings.guardrail_name.as_deref();
        let shown = name.unwrap_or("None");
        if settings.mask_request_content || settings.mask_response_content {
            tracing::warn!(
                "Guardrail {shown}: only_scan_new_messages is not supported with masking; scanning full context."
            );
            return None;
        }
        let Some(session_id) = session_id_from_request(request_data) else {
            tracing::debug!(
                "Guardrail {shown}: only_scan_new_messages enabled but request has no session id; scanning full context."
            );
            return None;
        };
        let stored = match cache.get(&scan_cache_key(name, &session_id)).await {
            Ok(stored) => stored.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(
                    "Guardrail {shown}: failed to read scanned-message cache ({error}); scanning full context."
                );
                return None;
            }
        };
        let seen: BTreeSet<&str> = stored.iter().map(String::as_str).collect();
        let hashes = scanned_text_hashes(present);
        Some(
            present
                .iter()
                .zip(&hashes)
                .filter(|(_text, hash)| !seen.contains(hash.as_str()))
                .map(|(text, _hash)| text.clone())
                .collect(),
        )
    }

    // The cache is best effort: a failed read or write only makes the next call scan again
    pub async fn mark_texts_scanned<S: ScanCacheStore>(
        &self,
        texts: Option<&[String]>,
        request_data: &BTreeMap<String, GuardrailValue>,
        cache: &S,
        ttl_seconds: i64,
    ) {
        let settings = &self.settings;
        let Some(present) = texts.filter(|list| !list.is_empty()) else {
            return;
        };
        if !settings.only_scan_new_messages
            || settings.mask_request_content
            || settings.mask_response_content
        {
            return;
        }
        let Some(session_id) = session_id_from_request(request_data) else {
            return;
        };
        let name = settings.guardrail_name.as_deref();
        let key = scan_cache_key(name, &session_id);
        let shown = name.unwrap_or("None");
        let stored = match cache.get(&key).await {
            Ok(stored) => stored.unwrap_or_default(),
            Err(error) => {
                tracing::warn!(
                    "Guardrail {shown}: failed to persist scanned-message cache ({error}); next call will re-scan."
                );
                return;
            }
        };
        let mut merged = Vec::new();
        let mut seen = BTreeSet::new();
        for hash in stored.into_iter().chain(scanned_text_hashes(present)) {
            if seen.insert(hash.clone()) {
                merged.push(hash);
            }
        }
        if let Err(error) = cache.set(&key, merged, ttl_seconds).await {
            tracing::warn!(
                "Guardrail {shown}: failed to persist scanned-message cache ({error}); next call will re-scan."
            );
        }
    }
}
