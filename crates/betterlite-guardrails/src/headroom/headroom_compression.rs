use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::ClientBuilder;
use thiserror::Error;
use uuid::Uuid;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_records::error::GuardrailRecordError;
use crate::headroom_messages::HeadroomMessagesError;
use crate::headroom_settings::{
    HeadroomSettings, HeadroomSettingsError, SUPPORTED_EVENT_HOOKS, resolve_timeout,
};

mod apply;
mod compress;
mod messages;
mod records;
mod request_data;
mod retrieve_tool;
mod tests;

// LiteLLM's _HASH_CACHE_TTL_SECONDS: 15 * 60
const HASH_CACHE_TTL: Duration = Duration::from_secs(900);

pub type Clock = Box<dyn Fn() -> Duration + Send + Sync>;

#[derive(Debug, Error)]
pub enum HeadroomError {
    #[error("the Headroom HTTP client could not be built")]
    Client {
        #[source]
        source: reqwest::Error,
    },
    #[error(transparent)]
    Settings(#[from] HeadroomSettingsError),
    #[error(transparent)]
    Messages(#[from] HeadroomMessagesError),
    #[error(transparent)]
    Record(#[from] GuardrailRecordError),
    #[error("a Headroom message value could not be read as JSON")]
    Json {
        #[from]
        source: serde_json::Error,
    },
}

impl From<HeadroomError> for GuardrailFailure {
    fn from(error: HeadroomError) -> Self {
        Self::Other(error.to_string())
    }
}

#[derive(Debug, Clone)]
struct IssuedHashes {
    hashes: BTreeSet<String>,
    expires_at: Duration,
}

#[derive(Debug, Default)]
struct IssuedHashStore {
    entries: Mutex<BTreeMap<String, IssuedHashes>>,
}

impl IssuedHashStore {
    fn prune_expired(&self, now: Duration) {
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|_call_id, issued| issued.expires_at > now);
    }

    fn record(&self, call_id: &str, hashes: BTreeSet<String>, now: Duration) {
        let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
        let issued = IssuedHashes {
            hashes,
            expires_at: now.saturating_add(HASH_CACHE_TTL),
        };
        entries.insert(call_id.to_owned(), issued);
    }
}

pub struct HeadroomGuardrail {
    pub guardrail_name: Option<String>,
    pub settings: HeadroomSettings,
    pub client: reqwest::Client,
    clock: Clock,
    new_id: fn() -> Uuid,
    issued: IssuedHashStore,
}

impl HeadroomGuardrail {
    pub fn new(
        guardrail_name: Option<String>,
        settings: HeadroomSettings,
        client_builder: ClientBuilder,
        clock: Clock,
        new_id: fn() -> Uuid,
    ) -> Result<Self, HeadroomError> {
        let connect = settings.timeout.connect;
        let client = client_builder
            .connect_timeout(connect)
            .build()
            .map_err(|source| HeadroomError::Client { source })?;
        Ok(Self {
            guardrail_name,
            settings,
            client,
            clock,
            new_id,
            issued: IssuedHashStore::default(),
        })
    }

    pub fn update_in_memory_params(&mut self, timeout: Option<f64>) {
        let resolved = resolve_timeout(timeout);
        tracing::debug!(
            total = ?resolved.total,
            connect = ?resolved.connect,
            "Headroom: compress timeout re-resolved"
        );
        self.settings.timeout = resolved;
    }

    pub fn has_issued_hashes(&self, call_id: &str) -> Option<BTreeSet<String>> {
        self.issued
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(call_id)
            .map(|issued| issued.hashes.clone())
    }
}

impl GuardrailContract for HeadroomGuardrail {
    fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> impl Future<Output = Result<GuardrailApiInputs, GuardrailFailure>> + Send {
        let target: Option<&GuardrailLogTarget> = logging_obj.map(|logging| &*logging);
        async move { self.apply(inputs, request_data, input_type, target).await }
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}
