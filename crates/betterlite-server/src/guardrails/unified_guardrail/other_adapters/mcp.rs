use std::collections::BTreeMap;
use std::marker::PhantomData;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_core::routes::CallType;
use betterlite_guardrails::generic_guardrail_contracts::GuardrailInputType;
use betterlite_guardrails::generic_guardrail_contracts::headers::LoggingObjectContext;
use betterlite_guardrails::guardrail_base::contract::GuardrailContract;
use betterlite_guardrails::guardrail_base::failure::GuardrailFailure;
use betterlite_guardrails::guardrail_base::inputs::{GuardrailApiInputs, GuardrailLogTarget};
use betterlite_guardrails::guardrail_translation::{BaseTranslation, TranslationCall};
use betterlite_guardrails::mcp_guardrail_translation::call_tool_result::CallToolResult;
use betterlite_guardrails::mcp_guardrail_translation::error::TranslationError;
use betterlite_guardrails::mcp_guardrail_translation::guardrail::{
    ApplyGuardrail, GuardrailInputs as McpInputs,
};
use betterlite_guardrails::mcp_guardrail_translation::tool_request::McpToolRequest;
use betterlite_guardrails::mcp_guardrail_translation::{process_input, process_output};
use indexmap::IndexMap;

use super::converted;

pub const MCP_CALL_TYPES: [CallType; 2] = [CallType::CallMcpTool, CallType::ListMcpTools];

pub struct McpTranslation<G>(pub PhantomData<fn() -> G>);

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
