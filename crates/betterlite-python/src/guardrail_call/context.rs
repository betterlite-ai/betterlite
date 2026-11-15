use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::invocation::GuardrailInvocation;
use betterlite_guardrails::guardrail_records::metadata::add_guardrail_record;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailRecordInput, GuardrailSpanSink,
};
use serde::{Deserialize, Serialize};

use super::RecordUpdate;
use super::convert;
use super::convert::settings::SettingsJson;
use super::error::CallError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextJson {
    settings: SettingsJson,
    #[serde(default)]
    log_target: Option<LogTargetJson>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct LogTargetJson {
    litellm_params: Option<BTreeMap<String, GuardrailValue>>,
    model_call_params: Option<BTreeMap<String, GuardrailValue>>,
}

impl LogTargetJson {
    fn into_target(self) -> Result<GuardrailLogTarget, CallError> {
        Ok(GuardrailLogTarget {
            litellm_params: self.litellm_params.map(convert::exact_data).transpose()?,
            model_call_params: self
                .model_call_params
                .map(convert::exact_data)
                .transpose()?,
        })
    }

    fn from_target(target: &GuardrailLogTarget) -> Self {
        Self {
            litellm_params: target.litellm_params.clone(),
            model_call_params: target.model_call_params.clone(),
        }
    }
}

struct NoSpanSink;

impl GuardrailSpanSink for NoSpanSink {
    fn emit(&self, _record: &GuardrailInvocationRecord) -> Result<(), GuardrailSpanError> {
        Ok(())
    }
}

pub(crate) struct RecordParts {
    base: GuardrailBase,
    sink: NoSpanSink,
    log_target: Option<GuardrailLogTarget>,
}

pub(crate) enum RecordStep {
    Add(GuardrailRecordInput),
    Own(GuardrailRecordInput),
    Finish(Option<GuardrailValue>),
}

impl RecordParts {
    pub(crate) fn context<'parts>(
        &'parts mut self,
        data: &'parts mut BTreeMap<String, GuardrailValue>,
    ) -> GuardrailRecordContext<'parts> {
        GuardrailRecordContext {
            base: &self.base,
            request_data: data,
            log_target: self.log_target.as_mut(),
            sink: &self.sink,
        }
    }

    pub(crate) fn recorded(
        &self,
        data: &BTreeMap<String, GuardrailValue>,
    ) -> Result<RecordUpdate, CallError> {
        Ok((
            serde_json::to_string(data).map_err(|source| CallError::Encode { source })?,
            self.log_target_text()?,
        ))
    }

    pub(crate) fn log_target_text(&self) -> Result<String, CallError> {
        self.log_target
            .as_ref()
            .map_or_else(|| Ok("null".to_owned()), log_target_to_text)
    }
}

pub(crate) fn record_parts(text: &str) -> Result<RecordParts, CallError> {
    let context: ContextJson = convert::read("guardrail record context", text)?;
    let settings = context.settings.into_settings();
    // the settings already passed the guardrail's own constructor, so validating them again would reject a guardrail built with strict modes off
    let base = GuardrailBase {
        settings,
        mode: None,
    };
    Ok(RecordParts {
        base,
        sink: NoSpanSink,
        log_target: context
            .log_target
            .map(LogTargetJson::into_target)
            .transpose()?,
    })
}

pub(crate) fn record_call(
    invocation: &mut Option<GuardrailInvocation>,
    step: RecordStep,
    data: &str,
    context: &str,
) -> Result<RecordUpdate, CallError> {
    let raw: BTreeMap<String, GuardrailValue> = convert::read("request data", data)?;
    let mut body = convert::exact_data(raw)?;
    let mut parts = record_parts(context)?;
    let mut record_context = parts.context(&mut body);
    let outcome = match step {
        RecordStep::Add(input) => add_guardrail_record(&mut record_context, input),
        RecordStep::Own(input) => invocation
            .as_mut()
            .ok_or(CallError::Finished)?
            .record_own(&mut record_context, input),
        RecordStep::Finish(response) => invocation
            .take()
            .ok_or(CallError::Finished)?
            .finish_success(&mut record_context, response.as_ref(), SystemTime::now()),
    };
    outcome.map_err(CallError::from)?;
    parts.recorded(&body)
}

pub(crate) fn log_target_from_text(text: &str) -> Result<GuardrailLogTarget, CallError> {
    convert::read::<LogTargetJson>("guardrail logging target object", text)
        .and_then(LogTargetJson::into_target)
}

pub(crate) fn log_target_to_text(target: &GuardrailLogTarget) -> Result<String, CallError> {
    serde_json::to_string(&LogTargetJson::from_target(target))
        .map_err(|source| CallError::Encode { source })
}
