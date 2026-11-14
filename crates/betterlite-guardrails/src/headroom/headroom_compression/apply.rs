use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use super::HeadroomGuardrail;
use super::call_id;
use super::compress::{CompressError, CompressRequest};
use super::error::HeadroomError;
use super::messages::{self, Messages};
use super::record::{self, Record};
use super::request_data;
use super::retrieve_tool;
use super::wire;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::failure::GuardrailFailure;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::guardrail_records::record::GuardrailStatus;
use crate::headroom_failure::CompressResult;
use crate::headroom_messages::{
    flatten_for_compression, restore_content_shapes, restore_protected_messages,
};
use crate::primitives::json_value::JsonValue;

struct Compressed {
    messages: Messages,
    result: CompressResult,
    started: f64,
    ended: f64,
}

impl HeadroomGuardrail {
    pub(crate) async fn apply(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&GuardrailLogTarget>,
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
            Self::record_unavailable(self.guardrail_name.as_deref(), request_data, &compressed);
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
            .or_else(|| request_data::model(request_data));
        let started = record::now();
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
                    .map(|(key, value)| (key.clone(), wire::guardrail_value(value)))
                    .collect();
                return Err(GuardrailFailure::HttpObject {
                    status: failure.status,
                    detail,
                });
            }
        };
        let ended = record::now();
        Ok(Compressed {
            messages,
            result,
            started,
            ended,
        })
    }

    fn finish_success(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        compressed: Compressed,
        logging_obj: Option<&GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, GuardrailFailure> {
        let rows = Self::restore(&compressed)?;
        self.record_success(request_data, &compressed);
        let hashes = if self.settings.ccr_retrieval {
            compressed.result.ccr_hashes.clone()
        } else {
            BTreeSet::new()
        };
        if hashes.is_empty() {
            let structured = messages::to_guardrail_messages(&rows)?;
            return Ok(GuardrailApiInputs {
                structured_messages: Some(structured),
                ..inputs
            });
        }
        self.record_session(request_data, logging_obj, hashes);
        let tools = retrieve_tool::merged(inputs.tools.as_deref());
        let structured = messages::to_guardrail_messages(&rows)?;
        Ok(GuardrailApiInputs {
            structured_messages: Some(structured),
            tools: Some(tools),
            ..inputs
        })
    }

    fn restore(compressed: &Compressed) -> Result<Vec<JsonValue>, GuardrailFailure> {
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
        Ok(rows)
    }

    fn record_success(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        compressed: &Compressed,
    ) {
        let stats = wire::guardrail_value(&JsonValue::Object(compressed.result.stats.clone()));
        append_record(
            request_data,
            Record {
                guardrail_name: self.guardrail_name.as_deref(),
                response: stats,
                status: GuardrailStatus::Success,
                start_time: compressed.started,
                end_time: compressed.ended,
            },
        );
        record::append_applied(request_data, self.guardrail_name.as_deref());
    }

    fn record_unavailable(
        guardrail_name: Option<&str>,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        compressed: &Compressed,
    ) {
        append_record(
            request_data,
            Record {
                guardrail_name,
                response: record::unavailable(),
                status: GuardrailStatus::GuardrailFailedToRespond,
                start_time: compressed.started,
                end_time: compressed.ended,
            },
        );
        record::append_applied(request_data, guardrail_name);
    }

    fn record_session(
        &self,
        request_data: &mut BTreeMap<String, GuardrailValue>,
        logging_obj: Option<&GuardrailLogTarget>,
        hashes: BTreeSet<String>,
    ) {
        self.issued.prune_expired((self.clock)());
        let (call_id, generated) = self.call_identity(logging_obj, request_data);
        if generated {
            request_data.insert(
                request_data::CALL_ID_FIELD.to_owned(),
                GuardrailValue::Text(call_id.clone()),
            );
        }
        self.issued.record(&call_id, hashes, (self.clock)());
    }

    fn call_identity(
        &self,
        logging_obj: Option<&GuardrailLogTarget>,
        request_data: &BTreeMap<String, GuardrailValue>,
    ) -> (String, bool) {
        if let Some(call_id) = call_id::resolve(logging_obj, request_data)
            && !call_id.is_empty()
        {
            return (call_id, false);
        }
        ((self.new_id)().to_string(), true)
    }
}

fn append_record(request_data: &mut BTreeMap<String, GuardrailValue>, invocation: Record<'_>) {
    if let Err(error) = record::append(request_data, invocation) {
        tracing::warn!(%error, "Headroom: guardrail record could not be written");
    }
}
