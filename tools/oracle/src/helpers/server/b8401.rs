mod tests;

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::request_guardrail_policy::GuardrailPolicySettings;
use betterlite_oracle_helper_api::error::{HelperError, OrHelperError as _};
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use betterlite_policy::registry::PolicyRegistry;
use betterlite_server::mcp::hook_payloads::from_llm::{
    llm_result_to_during_call, llm_result_to_pre_call,
};
use betterlite_server::mcp::hook_payloads::to_llm::{
    AuthGuardrails, McpGuardrailRequest, mcp_to_llm_request,
};
use betterlite_server::mcp::hook_payloads::{
    GuardrailHookData, GuardrailOutcome, McpHookKwargs, McpToolCall, ToolArguments,
};
use secrecy::ExposeSecret as _;
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) struct Op;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    scenarios: Vec<Scenario>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Scenario {
    function: Function,
    request: ToolRequest,
    #[serde(default)]
    kwargs: McpHookKwargs,
    #[serde(default)]
    premium: bool,
    #[serde(default)]
    result: Option<Outcome>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Function {
    ToLlm,
    PreCall,
    DuringCall,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolRequest {
    tool_name: String,
    arguments: ToolArguments,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Error(String),
    Text(String),
    Data(GuardrailHookData),
}

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        let views = input
            .scenarios
            .iter()
            .map(scenario_view)
            .collect::<Result<Vec<Value>, Box<HelperError>>>()?;
        Ok(Value::Array(views))
    }
}

fn scenario_view(scenario: &Scenario) -> HelperResult {
    let call = McpToolCall {
        tool_name: &scenario.request.tool_name,
        arguments: &scenario.request.arguments,
    };
    match scenario.function {
        Function::ToLlm => {
            let policies = PolicyRegistry::default();
            let guardrails = AuthGuardrails {
                policies: &policies,
                settings: GuardrailPolicySettings {
                    premium_user: scenario.premium,
                    limits: ApiLimits::default(),
                },
            };
            let request =
                mcp_to_llm_request(call, &scenario.kwargs, &guardrails).or_value_error()?;
            Ok(request_view(&request))
        }
        Function::PreCall => {
            let response = llm_result_to_pre_call(&outcome(scenario)?, call).or_value_error()?;
            Ok(response.map_or(Value::Null, |found| {
                json!({
                    "should_proceed": found.should_proceed,
                    "modified_arguments": found.modified_arguments,
                    "error_message": found.error_message,
                })
            }))
        }
        Function::DuringCall => {
            let response = llm_result_to_during_call(&outcome(scenario)?, call).or_value_error()?;
            Ok(response.map_or(Value::Null, |found| {
                json!({
                    "should_continue": found.should_continue,
                    "error_message": found.error_message,
                })
            }))
        }
    }
}

fn outcome(scenario: &Scenario) -> Result<GuardrailOutcome, Box<HelperError>> {
    match &scenario.result {
        Some(Outcome::Error(message)) => Ok(GuardrailOutcome::Raised(message.clone())),
        Some(Outcome::Text(text)) => Ok(GuardrailOutcome::Text(text.clone())),
        Some(Outcome::Data(data)) => Ok(GuardrailOutcome::Data(Box::new(data.clone()))),
        None => Err(Box::new(HelperError {
            class: "B8401".to_owned(),
            status: None,
            error_type: None,
            param: None,
            code: None,
            message: "pre_call and during_call scenarios need a result".to_owned(),
        })),
    }
}

// LiteLLM writes the description and input schema keys only when they are set
fn request_view(request: &McpGuardrailRequest) -> Value {
    let mut view = json!({
        "messages": request.messages,
        "model": request.model,
        "user_api_key_user_id": request.user_api_key_user_id,
        "user_api_key_team_id": request.user_api_key_team_id,
        "user_api_key_end_user_id": request.user_api_key_end_user_id,
        "user_api_key_hash": request.user_api_key_hash,
        "user_api_key_request_route": request.user_api_key_request_route,
        "mcp_tool_name": request.mcp_tool_name,
        "mcp_arguments": request.mcp_arguments,
        "mcp_server_name": request.mcp_server_name,
        "incoming_bearer_token": request
            .incoming_bearer_token
            .as_ref()
            .map(|token| token.expose_secret().to_owned()),
        "metadata": request.metadata,
    });
    if let Some(fields) = view.as_object_mut() {
        if let Some(description) = &request.mcp_tool_description {
            fields.insert("mcp_tool_description".to_owned(), json!(description));
        }
        if let Some(schema) = &request.mcp_input_schema {
            fields.insert("mcp_input_schema".to_owned(), json!(schema));
        }
    }
    view
}
