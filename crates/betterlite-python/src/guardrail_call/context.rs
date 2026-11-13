use std::collections::BTreeMap;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_guardrails::guardrail_base::GuardrailBase;
use betterlite_guardrails::guardrail_base::inputs::GuardrailLogTarget;
use betterlite_guardrails::guardrail_records::error::GuardrailSpanError;
use betterlite_guardrails::guardrail_records::record::{
    GuardrailInvocationRecord, GuardrailRecordContext, GuardrailSpanSink,
};
use serde::{Deserialize, Serialize};

use super::convert::exact_data;
use super::convert::settings::SettingsJson;
use super::error::GuardrailCallError;

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
    fn into_target(self) -> GuardrailLogTarget {
        GuardrailLogTarget {
            litellm_params: self.litellm_params.map(exact_data),
            model_call_params: self.model_call_params.map(exact_data),
        }
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

    pub(crate) fn log_target_text(&self) -> Result<String, GuardrailCallError> {
        self.log_target
            .as_ref()
            .map_or_else(|| Ok("null".to_owned()), log_target_to_text)
    }
}

pub(crate) fn record_parts(text: &str) -> Result<RecordParts, GuardrailCallError> {
    let context: ContextJson =
        serde_json::from_str(text).map_err(|source| GuardrailCallError::Json {
            field: "guardrail record context",
            source,
        })?;
    let settings = context.settings.into_settings()?;
    // the settings already passed the guardrail's own constructor, so validating them again here would reject a guardrail built with strict modes off
    let base = GuardrailBase {
        settings,
        mode: None,
    };
    Ok(RecordParts {
        base,
        sink: NoSpanSink,
        log_target: context.log_target.map(LogTargetJson::into_target),
    })
}

pub(crate) fn log_target_from_text(text: &str) -> Result<GuardrailLogTarget, GuardrailCallError> {
    let json: LogTargetJson =
        serde_json::from_str(text).map_err(|source| GuardrailCallError::Json {
            field: "guardrail log target",
            source,
        })?;
    Ok(json.into_target())
}

pub(crate) fn log_target_to_text(
    target: &GuardrailLogTarget,
) -> Result<String, GuardrailCallError> {
    let json = LogTargetJson::from_target(target);
    serde_json::to_string(&json).map_err(|source| GuardrailCallError::Json {
        field: "guardrail log target",
        source,
    })
}
