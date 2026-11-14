use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::HeadroomError;
use super::HeadroomGuardrail;
use super::compress::{CompressError, CompressRequest};
use super::messages::{self, Messages};
use super::records::guardrail_value;
use super::request_data;
use super::retrieve_tool;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::headroom_failure::CompressResult;
use crate::headroom_messages::{
    flatten_for_compression, restore_content_shapes, restore_protected_messages,
};

pub(crate) const UNAVAILABLE_MESSAGE: &str =
    "headroom compression unavailable; request forwarded uncompressed";

pub(super) struct Compressed {
    pub(super) messages: Messages,
    pub(super) result: CompressResult,
    pub(super) started: f64,
    pub(super) ended: f64,
}

impl HeadroomGuardrail {
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
        let compressed = self.compress(request_data, messages).await?;
        if !compressed.result.succeeded {
            self.record_outcome(request_data, &compressed, logging_obj);
            return Ok(inputs);
        }
        self.finish_success(inputs, request_data, compressed, logging_obj)
    }

    async fn compress(
        &self,
        request_data: &BTreeMap<String, GuardrailValue>,
        messages: Messages,
    ) -> Result<Compressed, GuardrailFailure> {
        let rows = flatten_for_compression(&messages.compressible);
        let model = self
            .settings
            .model
            .as_deref()
            .filter(|model| !model.is_empty())
            .or_else(|| request_data::model(request_data));
        let started = wall_seconds((self.runtime.wall)());
        let result = match self
            .call_compress(CompressRequest { rows: &rows, model })
            .await
        {
            Ok(result) => result,
            Err(CompressError::Headers(error)) => {
                return Err(GuardrailFailure::from(HeadroomError::from(error)));
            }
            Err(CompressError::Failure(failure)) => {
                let detail = failure
                    .body
                    .iter()
                    .map(|(key, value)| (key.clone(), guardrail_value(value)))
                    .collect();
                return Err(GuardrailFailure::HttpObject {
                    status: failure.status,
                    detail,
                });
            }
        };
        let ended = wall_seconds((self.runtime.wall)());
        Ok(Compressed {
            messages,
            result,
            started,
            ended,
        })
    }

    fn finish_success(
        &self,
        mut inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        compressed: Compressed,
        mut logging_obj: Option<&mut GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let shaped = restore_content_shapes(
            &compressed.messages.compressible,
            &compressed.result.messages,
        );
        let rows = restore_protected_messages(
            &compressed.messages.all,
            &shaped,
            &compressed.messages.protected,
        )
        .map_err(HeadroomError::from)?;
        let target = logging_obj.as_deref_mut();
        self.record_outcome(request_data, &compressed, target);
        let mut hashes = compressed.result.ccr_hashes.clone();
        if !self.settings.ccr_retrieval {
            hashes.clear();
        }
        let mut tools = inputs.tools.take();
        if !hashes.is_empty() {
            self.record_session(request_data, logging_obj.as_deref(), hashes);
            tools = Some(retrieve_tool::merged(tools.as_deref())?);
        }
        let structured = messages::convert(&rows)?;
        Ok(GuardrailApiInputs {
            structured_messages: Some(structured),
            tools,
            ..inputs
        })
    }

    fn record_session(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        logging_obj: Option<&GuardrailLogTarget>,
        hashes: BTreeSet<String>,
    ) {
        self.issued.prune_expired((self.runtime.clock)());
        let call_id = match request_data::resolve_call_id(logging_obj, request_data) {
            Some(call_id) if !call_id.is_empty() => call_id,
            _ => {
                let generated = (self.runtime.new_id)().to_string();
                request_data.insert(
                    request_data::CALL_ID_FIELD.to_owned(),
                    GuardrailValue::Text(generated.clone()),
                );
                generated
            }
        };
        self.issued.record(&call_id, hashes, (self.runtime.clock)());
    }
}

fn wall_seconds(time: SystemTime) -> f64 {
    let Ok(elapsed) = time.duration_since(UNIX_EPOCH) else {
        return 0.0;
    };
    elapsed.as_secs_f64()
}
