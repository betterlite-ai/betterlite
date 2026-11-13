mod call;
pub mod error;
pub mod init;
mod metadata;
mod payload;
pub mod settings;
mod tool_calls;

use std::collections::BTreeMap;
use std::time::Duration;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::guardrail_identity::item::GuardrailValue;
use betterlite_config::sections::guardrail_params::records::GuardrailEventHooks;

use crate::generic_guardrail_contracts::GuardrailInputType;
use crate::guardrail_base::inputs::GuardrailApiInputs;
use error::SingulrError;
use init::SingulrArgs;
use settings::{SettingsError, SingulrSettings};

pub type RequestData = BTreeMap<String, GuardrailValue>;

pub const DEFAULT_API_BASE: &str = "http://localhost:8003";
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
pub const GUARD_ENDPOINT: &str = "/api/v1/ai-gateway/litellm-v2";
pub const MAX_RESPONSE_BYTES: usize = 1024 * 1024;
pub const MCP_CALL_TYPE: &str = "call_mcp_tool";
pub const MCP_MODEL_PREFIX: &str = "MCP:";

// LiteLLM installs these hooks when the caller passes no supported_event_hooks of its own
pub const SUPPORTED_EVENT_HOOKS: [GuardrailEventHooks; 5] = [
    GuardrailEventHooks::PreCall,
    GuardrailEventHooks::PostCall,
    GuardrailEventHooks::LoggingOnly,
    GuardrailEventHooks::PreMcpCall,
    GuardrailEventHooks::PostMcpCall,
];

// LiteLLM reads logging_obj.call_type before request_data["call_type"]
#[derive(Debug, Clone, Copy, Default)]
pub struct SingulrLoggingObject<'a> {
    pub call_type: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct SingulrGuardrail {
    pub settings: SingulrSettings,
    pub client: reqwest::Client,
}

impl SingulrGuardrail {
    pub fn new<L>(
        args: &SingulrArgs,
        environment: &L,
        client: reqwest::Client,
    ) -> Result<Self, SettingsError>
    where
        L: EnvironmentLookup,
    {
        Ok(Self {
            settings: SingulrSettings::resolve(args, environment)?,
            client,
        })
    }

    pub async fn apply_guardrail(
        &self,
        inputs: GuardrailApiInputs,
        request_data: &RequestData,
        input_type: GuardrailInputType,
        logging_obj: Option<&SingulrLoggingObject<'_>>,
    ) -> Result<GuardrailApiInputs, SingulrError> {
        let mcp = is_mcp_call(request_data, logging_obj);
        let Some(body) = payload::build(&inputs, request_data, input_type, mcp) else {
            return Ok(inputs);
        };
        match call::send(&self.client, &self.settings, &body).await {
            Ok(answer) => {
                if answer.should_block {
                    return Err(SingulrError::Blocked {
                        due_to: answer
                            .blocking_due_to
                            .unwrap_or_else(|| "unknown".to_owned()),
                    });
                }
                Ok(inputs)
            }
            Err(error) => {
                error.log();
                if self.settings.block_on_error {
                    return Err(error);
                }
                Ok(inputs)
            }
        }
    }
}

fn is_mcp_call(request_data: &RequestData, logging_obj: Option<&SingulrLoggingObject<'_>>) -> bool {
    if let Some(logging) = logging_obj {
        if let Some(call_type) = logging.call_type {
            return call_type == MCP_CALL_TYPE;
        }
        return names_an_mcp_tool(request_data);
    }
    match request_data.get("call_type") {
        Some(GuardrailValue::Text(call_type)) => call_type == MCP_CALL_TYPE,
        Some(
            GuardrailValue::Boolean(_)
            | GuardrailValue::SignedInteger(_)
            | GuardrailValue::UnsignedInteger(_)
            | GuardrailValue::Decimal(_)
            | GuardrailValue::Array(_)
            | GuardrailValue::Object(_),
        ) => false,
        Some(GuardrailValue::Null) | None => names_an_mcp_tool(request_data),
    }
}

fn names_an_mcp_tool(request_data: &RequestData) -> bool {
    request_data.contains_key("mcp_tool_name")
        || matches!(
            request_data.get("model"),
            Some(GuardrailValue::Text(model)) if model.starts_with(MCP_MODEL_PREFIX)
        )
}

fn text_value(value: &GuardrailValue) -> Option<&str> {
    if let GuardrailValue::Text(text) = value {
        return Some(text);
    }
    None
}

mod tests;
