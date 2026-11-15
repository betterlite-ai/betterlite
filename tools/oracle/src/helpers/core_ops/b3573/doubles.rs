use std::convert::Infallible;
use std::future::{Future, ready};
use std::sync::{Mutex, PoisonError};

pub(super) struct ScenarioLogging {
    pub(super) config: Option<Json>,
}

impl PassthroughLogging for ScenarioLogging {
    fn passthrough_guardrails_config(&self) -> Option<&Json> {
        self.config.as_ref()
    }
}

// the no-op reply keeps the check-only contract visible; the calls are what the cases compare
pub(super) struct RecordingGuardrail {
    pub(super) name: Option<String>,
    pub(super) calls: Mutex<Vec<Value>>,
}

impl RecordingGuardrail {
    pub(super) fn recorded(&self) -> Vec<Value> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

impl PassthroughGuardrail for RecordingGuardrail {
    type Failure = Infallible;
    type LogTarget = ScenarioLogging;

    fn guardrail_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn apply_guardrail(
        &self,
        inputs: PassthroughGuardrailInputs,
        request_data: &mut RequestData,
        input_type: PassthroughInputType,
        _logging_obj: Option<&mut ScenarioLogging>,
    ) -> impl Future<Output = Result<PassthroughGuardrailInputs, Infallible>> + Send {
        let kind = match input_type {
            PassthroughInputType::Request => "request",
            PassthroughInputType::Response => "response",
        };
        let texts: Vec<Value> = inputs
            .texts
            .iter()
            .map(|text| normalised_text(text))
            .collect();
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(json!({
                "texts": texts,
                "model": inputs.model,
                "input_type": kind,
                "request_data": request_data,
            }));
        ready(Ok(inputs))
    }
}

// B3601's passthrough policy lives in betterlite-guardrails, which depends on betterlite-core
pub(super) struct Policy;

impl PassthroughPolicy for Policy {
    fn get_settings(
        &self,
        config: &Json,
        guardrail_name: &str,
    ) -> Option<PassThroughGuardrailSettings> {
        passthrough_policy::get_settings(Some(config), guardrail_name).map(|settings| {
            PassThroughGuardrailSettings {
                request_fields: settings.request_fields,
                response_fields: settings.response_fields,
            }
        })
    }

    fn extract_fields(&self, data: &Json, expressions: &[String]) -> String {
        passthrough_policy::extract_fields(data, expressions)
    }

    fn user_key_metadata(&self, user_api_key_dict: Option<&RequestData>) -> RequestData {
        guardrail_translation::transform_user_key_metadata(user_api_key_dict)
    }
}
