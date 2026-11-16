use std::collections::BTreeMap;
use std::sync::PoisonError;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use indexmap::IndexMap;
use select::Plan;

use crate::compresr_recovery::store::{Originals, RecoveryStore, StoreKey};
use crate::compresr_recovery::tools::merged_tools;
use crate::compresr_settings::{CompresrGuardrail, UI_FRIENDLY_NAME};
use crate::compresr_transport::error::CompresrTransportError;
use crate::compresr_transport::{CompressOutcome, CompressionResult};
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::contract::GuardrailContract;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};

mod mirror;
mod records;
mod results;
mod scope;
mod select;
mod tests;

const NO_SCOPE_WARNING_INTERVAL: Duration = Duration::from_mins(15);
const CONFIG_MODEL: &str = "CompresrGuardrailConfigModel";

// The service's answer for one batch, the plan it answered and the clock readings around the call
struct Compressed<'call> {
    plan: &'call Plan,
    results: &'call [CompressionResult],
    started: Duration,
    ended: Duration,
}

struct Recovery<'store> {
    store: &'store RecoveryStore,
    key: StoreKey,
    existing: Originals,
}

// A failed compression is a 502 with the generic message, never the upstream body or the API key
fn failure(error: &CompresrTransportError) -> GuardrailFailure {
    match error.status_code() {
        Some(status) => GuardrailFailure::HttpObject {
            status,
            detail: IndexMap::from([("error".to_owned(), GuardrailValue::Text(error.to_string()))]),
        },
        None => GuardrailFailure::Other(error.to_string()),
    }
}

impl CompresrGuardrail {
    // None when nothing is to be compressed: another hook, the bypass header, no messages or no eligible one
    fn plan(
        &self,
        inputs: &GuardrailApiInputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
    ) -> Option<Plan> {
        if input_type != GuardrailInputType::Request || self.should_bypass(request_data) {
            return None;
        }
        let messages = inputs
            .structured_messages
            .as_deref()
            .filter(|rows| !rows.is_empty())?;
        let plan = self.plan_targets(messages);
        (!plan.targets.is_empty()).then_some(plan)
    }

    pub async fn apply(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let Some(plan) = self.plan(&inputs, request_data, input_type) else {
            return Ok(inputs);
        };
        let started = (self.runtime.clock)();
        let outcome = self
            .transport
            .compress(&plan.targets)
            .await
            .map_err(|error| failure(&error))?;
        let ended = (self.runtime.clock)();
        let CompressOutcome::Results(results) = outcome else {
            return Ok(inputs);
        };
        let compressed = Compressed {
            plan: &plan,
            results: &results,
            started,
            ended,
        };
        self.finish(inputs, request_data, logging_obj, &compressed)
    }

    fn finish(
        &self,
        mut inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        logging_obj: Option<&mut GuardrailLogTarget>,
        compressed: &Compressed<'_>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let now = (self.runtime.clock)();
        let recovery = self.recovery_for(logging_obj.as_deref(), now);
        let rows = inputs.structured_messages.take().unwrap_or_default();
        let existing = recovery.as_ref().map(|active| &active.existing);
        let applied = self.apply_compression_results(rows, compressed, existing);
        // Untouched inputs go back as they came, so a handler that detects edits by comparison sees none
        if applied.messages_compressed == 0 {
            tracing::debug!("Compresr: service returned no compressed content; request unchanged");
            return Ok(GuardrailApiInputs {
                structured_messages: Some(applied.messages),
                ..inputs
            });
        }
        self.record_success(request_data, logging_obj, &applied, compressed);
        let mirrored = mirror::mirror_texts_channel(inputs.texts.as_deref(), &applied.replacements);
        let mut tools = inputs.tools.take();
        if let Some(active) = &recovery
            && !applied.originals.is_empty()
        {
            active.store.store(
                &active.key,
                applied.originals,
                now,
                self.settings.max_bytes_per_call,
            );
            tools = Some(
                merged_tools(tools.as_deref())
                    .map_err(|error| GuardrailFailure::Other(error.to_string()))?,
            );
        }
        Ok(GuardrailApiInputs {
            texts: mirrored.or(inputs.texts),
            tools,
            structured_messages: Some(applied.messages),
            ..inputs
        })
    }

    // Recovery needs a per-tenant scope, since the call id alone is client-settable, and a call id to file the originals under
    fn recovery_for(
        &self,
        log: Option<&GuardrailLogTarget>,
        now: Duration,
    ) -> Option<Recovery<'_>> {
        let store = self.recovery.as_ref()?;
        let scope = scope::caller_scope(log);
        if scope.is_empty() {
            self.warn_without_scope(now);
            return None;
        }
        let key = StoreKey::resolve(scope::call_id(log), &scope)?;
        let existing = store.existing_originals(Some(&key));
        Some(Recovery {
            store,
            key,
            existing,
        })
    }

    // Once per interval, so a standing misconfiguration stays visible without flooding the log
    fn warn_without_scope(&self, now: Duration) {
        let mut until = self
            .no_scope_warning_until
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if now < *until {
            return;
        }
        *until = now.saturating_add(NO_SCOPE_WARNING_INTERVAL);
        tracing::warn!(
            "Compresr: enable_retrieval is on but this request has no per-key auth scope; compressing without recovery (compresr_retrieve tool not injected). Configure virtual-key auth to enable recovery."
        );
    }
}

impl GuardrailContract for CompresrGuardrail {
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

    fn get_config_model(&self) -> Option<GuardrailValue> {
        let names = [
            ("model", CONFIG_MODEL),
            ("ui_friendly_name", UI_FRIENDLY_NAME),
        ];
        Some(GuardrailValue::Object(BTreeMap::from(names.map(
            |(key, name)| (key.to_owned(), GuardrailValue::Text(name.to_owned())),
        ))))
    }
}
