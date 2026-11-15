use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;
use reqwest::Client;

use crate::content_text::content_to_text;
use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::GuardrailBase;
use crate::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use crate::hiddenlayer::evaluation::error::EvaluationError;
use crate::hiddenlayer::evaluation::metadata;
use crate::hiddenlayer::evaluation::wire;
use crate::hiddenlayer::evaluation::{
    EvaluationArgs, INTERACTIONS_PATH, detection_headers, evaluation_timeout, guardrail_settings,
};
use crate::hiddenlayer::hiddenlayer_authentication::HiddenlayerAuth;
use crate::hiddenlayer::hiddenlayer_authentication::detection::DetectionRequest;
use crate::hiddenlayer::hiddenlayer_authentication::token::JwtTransport;
use crate::primitives::json_value::JsonValue;

mod response;
mod tests;

use response::{InteractionBody, apply_result, interaction_body};

pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 2] =
    [GuardrailEventHooks::PreCall, GuardrailEventHooks::PostCall];
pub const VERSION: &str = "1";

// LiteLLM's HiddenlayerGuardrail: one scan of the last user message per call
pub struct HiddenlayerV1Guardrail {
    pub base: GuardrailBase,
    pub auth: HiddenlayerAuth,
    pub timeout: Duration,
}

impl HiddenlayerV1Guardrail {
    pub fn new<L: EnvironmentLookup>(
        args: &EvaluationArgs,
        environment: &L,
        client: Client,
        transport: Arc<dyn JwtTransport>,
    ) -> Result<Self, EvaluationError> {
        let auth = HiddenlayerAuth::new(&args.credentials(), environment, client, transport)?;
        let timeout = evaluation_timeout(args.timeout)?;
        let settings = guardrail_settings(args, Some(SUPPORTED_EVENT_HOOKS.to_vec()));
        let base = GuardrailBase::new(settings, true)?;
        Ok(Self {
            base,
            auth,
            timeout,
        })
    }

    pub async fn evaluate(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &BTreeMap<String, GuardrailValue>,
        input_type: GuardrailInputType,
        logging_obj: Option<&GuardrailLogTarget>,
    ) -> Result<GuardrailApiInputs, EvaluationError> {
        let headers = metadata::request_headers(request_data, logging_obj);
        let Some(payload) = scan_payload(&inputs) else {
            return Ok(inputs);
        };
        let project_id = headers
            .get(metadata::PROJECT_HEADER)
            .filter(|value| !value.is_empty())
            .map(String::as_str);
        let requester_id = headers
            .get(metadata::REQUESTER_HEADER)
            .filter(|value| !value.is_empty())
            .map_or(metadata::DEFAULT_REQUESTER, String::as_str);
        let metadata_body = BTreeMap::from([
            ("model".to_owned(), metadata::evaluation_model(request_data)),
            ("requester_id".to_owned(), requester_id.to_owned()),
        ]);
        let result = self
            .call_hiddenlayer(project_id, &metadata_body, &payload, input_type)
            .await?;
        apply_result(inputs, &result, input_type)
    }

    async fn call_hiddenlayer(
        &self,
        project_id: Option<&str>,
        metadata_body: &BTreeMap<String, String>,
        payload: &JsonValue,
        input_type: GuardrailInputType,
    ) -> Result<InteractionBody, EvaluationError> {
        let mut body = vec![("metadata", wire::to_json(metadata_body)?)];
        match input_type {
            GuardrailInputType::Request => body.push(("input", payload.clone())),
            GuardrailInputType::Response => body.push(("output", payload.clone())),
        }
        let body = wire::object(body);
        let request = DetectionRequest {
            path: INTERACTIONS_PATH,
            headers: detection_headers(VERSION, project_id, None)?,
            body: &body,
        };
        let response = self
            .auth
            .post_detection_with_timeout(&request, Some(self.timeout))
            .await?;
        interaction_body(&response.body)
    }
}

// LiteLLM scans structured_messages first and falls back to the last text
#[must_use]
pub fn scan_payload(inputs: &GuardrailApiInputs) -> Option<JsonValue> {
    if let Some(messages) = inputs.structured_messages.as_deref()
        && let Some(message) = messages.last()
    {
        let content = message
            .extra
            .get("content")
            .map_or_else(String::new, content_to_text);
        return Some(messages_payload(&message.role, &content));
    }
    if let Some(last) = inputs.texts.as_deref().and_then(|texts| texts.last()) {
        return Some(messages_payload("user", last));
    }
    None
}

fn messages_payload(role: &str, content: &str) -> JsonValue {
    wire::object(vec![(
        "messages",
        wire::array(vec![wire::object(vec![
            ("role", wire::text(role)),
            ("content", wire::text(content)),
        ])]),
    )])
}
