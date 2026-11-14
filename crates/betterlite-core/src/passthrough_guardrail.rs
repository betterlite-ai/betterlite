use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_logging::private_json::safe_json::{DEFAULT_MAX_RECURSE_DEPTH, safe_dumps};
use betterlite_models::decisions::Json;

pub mod errors;
pub mod payload;
pub mod settings;
pub mod traits;

mod tests;

use errors::{PassthroughGuardrailError, PassthroughPayloadError};
use payload::payload_json;
use settings::{
    PassThroughGuardrailSettings, PassthroughGuardrailInputs, PassthroughInputType, RequestData,
    ResponseCall,
};
use traits::{PassthroughGuardrail, PassthroughLogging, PassthroughPolicy};

pub struct PassThroughEndpointHandler;

impl PassThroughEndpointHandler {
    // LiteLLM's _get_guardrail_settings; the logging object carries the endpoint's guardrails config
    #[must_use]
    pub fn get_guardrail_settings<Log, Policy>(
        logging_obj: Option<&Log>,
        guardrail_name: Option<&str>,
        policy: &Policy,
    ) -> Option<PassThroughGuardrailSettings>
    where
        Log: PassthroughLogging,
        Policy: PassthroughPolicy,
    {
        let config = logging_obj?.passthrough_guardrails_config()?;
        let name = guardrail_name.filter(|candidate| !candidate.is_empty())?;
        if !carries_guardrails(config) {
            return None;
        }
        policy.get_settings(config, name)
    }

    // LiteLLM's _extract_text_for_guardrail; the extractor is B3601's passthrough policy
    #[must_use]
    pub fn extract_text_for_guardrail<Policy>(
        data: &RequestData,
        field_expressions: Option<&[String]>,
        policy: &Policy,
    ) -> Result<String, PassthroughPayloadError>
    where
        Policy: PassthroughPolicy,
    {
        if let Some(expressions) = field_expressions.filter(|expressions| !expressions.is_empty()) {
            return Ok(policy.extract_fields(&payload_json(data), expressions));
        }
        let payload_to_check = data
            .iter()
            .filter(|(key, _value)| {
                !key.starts_with('_') && *key != "metadata" && *key != "litellm_logging_obj"
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<RequestData>();
        safe_dumps(&payload_to_check, DEFAULT_MAX_RECURSE_DEPTH, None)
            .map_err(PassthroughPayloadError::Serialize)
    }

    pub async fn process_input_messages<Guardrail, Policy>(
        data: &mut RequestData,
        guardrail: &Guardrail,
        logging_obj: Option<&mut Guardrail::LogTarget>,
        policy: &Policy,
    ) -> Result<(), PassthroughGuardrailError<Guardrail::Failure>>
    where
        Guardrail: PassthroughGuardrail,
        Policy: PassthroughPolicy,
    {
        let settings = Self::get_guardrail_settings(
            logging_obj.as_deref(),
            guardrail.guardrail_name(),
            policy,
        );
        let field_expressions = settings
            .as_ref()
            .and_then(|settings| settings.request_fields.as_deref());
        let text_to_check = Self::extract_text_for_guardrail(data, field_expressions, policy)
            .map_err(PassthroughGuardrailError::Payload)?;
        if text_to_check.is_empty() {
            tracing::debug!("PassThroughEndpointHandler: No text to check, skipping guardrail");
            return Ok(());
        }
        let inputs = PassthroughGuardrailInputs {
            texts: vec![text_to_check],
            model: model_from(data.get("model")),
        };
        guardrail
            .apply_guardrail(inputs, data, PassthroughInputType::Request, logging_obj)
            .await
            .map_err(PassthroughGuardrailError::Guardrail)?;
        Ok(())
    }

    pub async fn process_output_response<Guardrail, Policy>(
        response: GuardrailValue,
        guardrail: &Guardrail,
        call: ResponseCall<'_, Guardrail::LogTarget>,
        policy: &Policy,
    ) -> Result<GuardrailValue, PassthroughGuardrailError<Guardrail::Failure>>
    where
        Guardrail: PassthroughGuardrail,
        Policy: PassthroughPolicy,
    {
        let GuardrailValue::Object(fields) = &response else {
            tracing::debug!("PassThroughEndpointHandler: Response is not a dict, skipping");
            return Ok(response);
        };
        let text_to_check =
            response_scan_text(fields, guardrail, call.logging_obj.as_deref(), policy)?;
        if text_to_check.is_empty() {
            return Ok(response);
        }
        apply_response_guardrail(fields, text_to_check, guardrail, call, policy).await?;
        Ok(response)
    }
}

async fn apply_response_guardrail<Guardrail, Policy>(
    fields: &RequestData,
    text_to_check: String,
    guardrail: &Guardrail,
    call: ResponseCall<'_, Guardrail::LogTarget>,
    policy: &Policy,
) -> Result<(), PassthroughGuardrailError<Guardrail::Failure>>
where
    Guardrail: PassthroughGuardrail,
    Policy: PassthroughPolicy,
{
    let ResponseCall {
        logging_obj,
        user_api_key_dict,
        request_data,
    } = call;
    let mut standalone;
    let prepared = match request_data {
        Some(caller_data) => {
            if !caller_data.contains_key("response") {
                caller_data.insert(
                    "response".to_owned(),
                    GuardrailValue::Object(fields.clone()),
                );
            }
            caller_data
        }
        None => {
            standalone = fields.clone();
            &mut standalone
        }
    };
    if !prepared.contains_key("litellm_metadata") {
        let metadata = policy.user_key_metadata(user_api_key_dict);
        if !metadata.is_empty() {
            prepared.insert(
                "litellm_metadata".to_owned(),
                GuardrailValue::Object(metadata),
            );
        }
    }
    let inputs = PassthroughGuardrailInputs {
        texts: vec![text_to_check],
        model: model_from(fields.get("model")),
    };
    guardrail
        .apply_guardrail(
            inputs,
            prepared,
            PassthroughInputType::Response,
            logging_obj,
        )
        .await
        .map_err(PassthroughGuardrailError::Guardrail)?;
    Ok(())
}

fn response_scan_text<Guardrail, Policy>(
    fields: &RequestData,
    guardrail: &Guardrail,
    logging_obj: Option<&Guardrail::LogTarget>,
    policy: &Policy,
) -> Result<String, PassthroughGuardrailError<Guardrail::Failure>>
where
    Guardrail: PassthroughGuardrail,
    Policy: PassthroughPolicy,
{
    let settings = PassThroughEndpointHandler::get_guardrail_settings(
        logging_obj,
        guardrail.guardrail_name(),
        policy,
    );
    let field_expressions = settings
        .as_ref()
        .and_then(|settings| settings.response_fields.as_deref());
    PassThroughEndpointHandler::extract_text_for_guardrail(fields, field_expressions, policy)
        .map_err(PassthroughGuardrailError::Payload)
}

// Python's `if not passthrough_config`: an empty list or object leaves the endpoint without guardrails
fn carries_guardrails(config: &Json) -> bool {
    match config {
        Json::Null => false,
        Json::List(items) => !items.is_empty(),
        Json::Object(fields) => !fields.is_empty(),
        Json::Bool(_) | Json::Number(_) | Json::Text(_) => true,
    }
}

fn model_from(value: Option<&GuardrailValue>) -> Option<String> {
    let Some(GuardrailValue::Text(text)) = value else {
        return None;
    };
    if text.is_empty() {
        return None;
    }
    Some(text.clone())
}
