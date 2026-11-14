use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::ClientBuilder;
use uuid::Uuid;

use self::error::HeadroomError;
use self::hashes::IssuedHashStore;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::headroom_settings::{HeadroomSettings, SUPPORTED_EVENT_HOOKS, resolve_timeout};

mod apply;
mod call_id;
mod compress;
pub mod error;
mod hashes;
mod messages;
mod record;
mod request_data;
mod retrieve_tool;
mod tests;
mod wire;

pub type Clock = Box<dyn Fn() -> Duration + Send + Sync>;

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
        self.issued.lookup(call_id)
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
