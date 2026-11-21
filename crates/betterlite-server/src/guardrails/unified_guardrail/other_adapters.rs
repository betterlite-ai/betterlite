use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::generic_guardrail_contracts::headers::LoggingObjectContext;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_translation::cohere_rerank_guardrail::{
    CohereRerankGuardrailError, CohereRerankHandler,
};
use betterlite_guardrails::guardrail_translation::{
    BaseTranslation, TranslationCall, transform_user_key_metadata,
};
use betterlite_guardrails::mcp_guardrail_translation::call_tool_result::CallToolResult;
use betterlite_guardrails::mcp_guardrail_translation::error::TranslationError;
use betterlite_guardrails::mcp_guardrail_translation::guardrail::{
    ApplyGuardrail, GuardrailInputs as McpInputs,
};
use betterlite_guardrails::mcp_guardrail_translation::tool_request::McpToolRequest;
use betterlite_guardrails::mcp_guardrail_translation::{process_input, process_output};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_vertex::google_genai_guardrail::CALL_TYPES;
use betterlite_vertex::google_genai_guardrail::callbacks::{
    ApplyFuture, GenaiGuardrail, GuardedOutput, GuardrailCall, GuardrailInputs as GenaiInputs,
    InputType, process_output, process_request,
};
use indexmap::IndexMap;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub const RERANK_CALL_TYPES: [CallType; 2] = [CallType::Rerank, CallType::Arerank];

pub const GOOGLE_GENAI_CALL_TYPES: [CallType; 4] = CALL_TYPES;

pub const MCP_CALL_TYPES: [CallType; 2] = [CallType::CallMcpTool, CallType::ListMcpTools];

pub struct RerankTranslation<G>(pub PhantomData<fn() -> G>);

pub struct GoogleGenAiTranslation<G>(pub PhantomData<fn() -> G>);

pub struct McpTranslation<G>(pub PhantomData<fn() -> G>);

fn converted<Source, Target>(value: &Source) -> Result<Target, GuardrailFailure>
where
    Source: Serialize + ?Sized,
    Target: DeserializeOwned,
{
    serde_json::to_value(value)
        .and_then(serde_json::from_value)
        .map_err(|error| {
            GuardrailFailure::Other(format!("guardrail data could not be converted: {error}"))
        })
}

impl<G: GuardrailContract> BaseTranslation for RerankTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        CohereRerankHandler
            .process_input_messages(data, guardrail, logging_obj)
            .await
            .map_err(|CohereRerankGuardrailError::Guardrail(failure)| failure)
    }

    async fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        _call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        Ok(CohereRerankHandler.process_output_response(response, guardrail))
    }
}

// B1391's handler speaks its own guardrail trait over prompt JSON, so each call crosses through this bridge
struct GenaiBridge<'call, G> {
    guardrail: &'call G,
    logging_obj: Option<&'call mut GuardrailLogTarget>,
}

impl<G: GuardrailContract> GenaiGuardrail for GenaiBridge<'_, G> {
    type Failure = GuardrailFailure;

    fn apply_guardrail<'call>(
        &'call mut self,
        inputs: GenaiInputs,
        call: GuardrailCall<'call>,
    ) -> ApplyFuture<'call, Self::Failure> {
        Box::pin(async move {
            let mut request_data: BTreeMap<String, GuardrailValue> = converted(call.request_data)?;
            let input_type = match call.input_type {
                InputType::Request => GuardrailInputType::Request,
                InputType::Response => GuardrailInputType::Response,
            };
            let sent = GuardrailApiInputs {
                texts: Some(inputs.texts),
                images: None,
                tools: None,
                tool_calls: None,
                structured_messages: None,
                model: inputs.model,
                stream_holdback_chars: None,
            };
            let returned = self
                .guardrail
                .apply_guardrail(
                    sent,
                    &mut request_data,
                    input_type,
                    self.logging_obj.as_deref_mut(),
                )
                .await?;
            Ok(returned.texts)
        })
    }
}

fn genai_context(
    call: &TranslationCall<'_, GuardrailLogTarget>,
) -> Result<(Option<Object>, Object), GuardrailFailure> {
    let request = match call.request_data.as_deref() {
        Some(data) => Some(converted(data)?),
        None => None,
    };
    let metadata = converted(&transform_user_key_metadata(call.user_api_key_dict))?;
    Ok((request, metadata))
}

impl<G: GuardrailContract> BaseTranslation for GoogleGenAiTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let mut request: Object = converted(&*data)?;
        let mut bridge = GenaiBridge {
            guardrail,
            logging_obj,
        };
        process_request(&mut request, &mut bridge).await?;
        *data = converted(&request)?;
        Ok(())
    }

    async fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        if !matches!(response, GuardrailValue::Object(_)) {
            return Ok(response);
        }
        let mut body: Object = converted(&response)?;
        let (request, metadata) = genai_context(&call)?;
        let mut bridge = GenaiBridge {
            guardrail,
            logging_obj: call.logging_obj,
        };
        let output = GuardedOutput::Response(&mut body);
        process_output(output, request.as_ref(), &metadata, &mut bridge).await?;
        converted(&body)
    }

    async fn process_streaming_response(
        &self,
        responses_so_far: &mut Vec<GuardrailValue>,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let frames: Vec<Json> = converted(&*responses_so_far)?;
        let (request, metadata) = genai_context(&call)?;
        let mut bridge = GenaiBridge {
            guardrail,
            logging_obj: call.logging_obj,
        };
        let output = GuardedOutput::Frames(&frames);
        process_output(output, request.as_ref(), &metadata, &mut bridge).await
    }
}

// B3818's handler takes its own guardrail trait over the MCP request; guardrail edits to the request are written back
struct McpBridge<'call, G> {
    guardrail: &'call G,
}

impl<G: GuardrailContract> ApplyGuardrail for McpBridge<'_, G> {
    type Error = GuardrailFailure;

    async fn apply_guardrail(
        &self,
        inputs: McpInputs,
        request_data: Option<&mut McpToolRequest>,
        input_type: GuardrailInputType,
        _logging_obj: Option<&mut LoggingObjectContext>,
    ) -> Result<Option<Vec<String>>, Self::Error> {
        let mut data: BTreeMap<String, GuardrailValue> = match request_data.as_deref() {
            Some(request) => converted(request)?,
            None => BTreeMap::new(),
        };
        let sent = GuardrailApiInputs {
            texts: Some(inputs.texts),
            images: None,
            tools: Some(converted(&inputs.tools)?),
            tool_calls: None,
            structured_messages: None,
            model: None,
            stream_holdback_chars: None,
        };
        let returned = self
            .guardrail
            .apply_guardrail(sent, &mut data, input_type, None)
            .await?;
        if let Some(request) = request_data {
            *request = converted(&data)?;
        }
        Ok(returned.texts)
    }
}

// LiteLLM's _blocked raises HTTPException(400, detail={"error": ...})
fn mcp_failure(error: TranslationError<GuardrailFailure>) -> GuardrailFailure {
    match error {
        TranslationError::Guardrail(failure) => failure,
        TranslationError::Blocked(blocked) => GuardrailFailure::HttpObject {
            status: 400,
            detail: IndexMap::from([(
                "error".to_owned(),
                GuardrailValue::Text(blocked.to_string()),
            )]),
        },
    }
}

impl<G: GuardrailContract> BaseTranslation for McpTranslation<G> {
    type Guardrail = G;
    type LogTarget = GuardrailLogTarget;
    type Failure = GuardrailFailure;

    async fn process_input_messages(
        &self,
        data: &mut BTreeMap<String, GuardrailValue>,
        guardrail: &Self::Guardrail,
        _logging_obj: Option<&mut Self::LogTarget>,
    ) -> Result<(), Self::Failure> {
        let mut request: McpToolRequest = converted(&*data)?;
        process_input::process_input_messages(&mut request, &McpBridge { guardrail }, None)
            .await
            .map_err(mcp_failure)?;
        *data = converted(&request)?;
        Ok(())
    }

    async fn process_output_response(
        &self,
        response: GuardrailValue,
        guardrail: &Self::Guardrail,
        call: TranslationCall<'_, Self::LogTarget>,
    ) -> Result<GuardrailValue, Self::Failure> {
        let mut result: CallToolResult = converted(&response)?;
        let mut request: Option<McpToolRequest> = match call.request_data.as_deref() {
            Some(data) => Some(converted(data)?),
            None => None,
        };
        let bridge = McpBridge { guardrail };
        process_output::process_output_response(&mut result, &bridge, request.as_mut(), None)
            .await
            .map_err(mcp_failure)?;
        if let (Some(data), Some(written)) = (call.request_data, request) {
            *data = converted(&written)?;
        }
        converted(&result)
    }
}

mod tests;
