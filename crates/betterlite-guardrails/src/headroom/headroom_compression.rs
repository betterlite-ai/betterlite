use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, SystemTime};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
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
const HASH_CACHE_TTL: Duration = Duration::from_mins(15);

pub type Clock = Box<dyn Fn() -> Duration + Send + Sync>;

// reqwest fixes connect_timeout when a client is built, so a new deadline needs a freshly built client
pub type ClientFactory = Box<dyn Fn(Duration) -> reqwest::Result<reqwest::Client> + Send + Sync>;

#[derive(Debug, Error)]
pub enum HeadroomError {
    #[error("the Headroom HTTP client could not be built")]
    Client(#[source] reqwest::Error),
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
        let issued = IssuedHashes {
            hashes,
            expires_at: now.saturating_add(HASH_CACHE_TTL),
        };
        self.entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(call_id.to_owned(), issued);
    }
}

pub struct HeadroomRuntime {
    pub clock: Clock,
    pub wall: fn() -> SystemTime,
    pub new_id: fn() -> Uuid,
    pub event_hook: Option<LitellmParamsMode>,
}

pub struct HeadroomGuardrail {
    guardrail_name: Option<String>,
    settings: HeadroomSettings,
    client: reqwest::Client,
    client_factory: ClientFactory,
    runtime: HeadroomRuntime,
    issued: IssuedHashStore,
}

impl HeadroomGuardrail {
    pub fn new(
        guardrail_name: Option<String>,
        settings: HeadroomSettings,
        client_factory: ClientFactory,
        runtime: HeadroomRuntime,
    ) -> Result<Self, HeadroomError> {
        let client = client_factory(settings.timeout.connect).map_err(HeadroomError::Client)?;
        Ok(Self {
            guardrail_name,
            settings,
            client,
            client_factory,
            runtime,
            issued: IssuedHashStore::default(),
        })
    }

    pub fn update_in_memory_params(&mut self, timeout: Option<f64>) -> Result<(), HeadroomError> {
        let resolved = resolve_timeout(timeout);
        let client = (self.client_factory)(resolved.connect).map_err(HeadroomError::Client)?;
        self.settings.timeout = resolved;
        self.client = client;
        Ok(())
    }

    pub fn has_issued_hashes(&self, call_id: &str) -> Option<BTreeSet<String>> {
        let entries = self
            .issued
            .entries
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        entries.get(call_id).map(|issued| issued.hashes.clone())
    }

    pub(crate) async fn apply(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        if input_type != GuardrailInputType::Request
            || request_data::bypass(request_data)
            || request_data::background(request_data)
        {
            return Ok(inputs);
        }
        let Some(messages) = messages::collect(&inputs, request_data)? else {
            return Ok(inputs);
        };
        let compressed = apply::compress_rows(self, request_data, messages).await?;
        if !compressed.result.succeeded {
            records::record_outcome(self, request_data, &compressed, logging_obj);
            return Ok(inputs);
        }
        apply::finish_success(self, inputs, request_data, &compressed, logging_obj)
    }
}

impl GuardrailContract for HeadroomGuardrail {
    async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        self.apply(inputs, request_data, input_type, logging_obj)
            .await
    }

    fn get_supported_event_hooks(&self) -> Option<Vec<GuardrailEventHooks>> {
        Some(SUPPORTED_EVENT_HOOKS.to_vec())
    }
}
