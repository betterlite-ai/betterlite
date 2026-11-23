use betterlite_core::api::responses_create::execute::{
    has_file_search_tool, mock_responses_api_response,
};
use betterlite_models::controls::json_value::JsonValue;
use betterlite_oracle_helper_api::error::OrHelperError as _;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde::Deserialize;
use serde_json::{Value, json};
#[derive(Debug, Deserialize)]
pub(crate) struct LiteralInput {
    steps: Vec<LiteralStep>,
}
#[derive(Debug, Deserialize)]
struct LiteralStep {
    text: Option<String>,
    #[serde(default)]
    tools: Vec<Option<Vec<JsonValue>>>,
}
pub(crate) struct Op;
impl Helper for Op {
    type Input = LiteralInput;
    fn run(input: &Self::Input) -> HelperResult {
        let mut steps = Vec::with_capacity(input.steps.len());
        for step in &input.steps {
            let response = mock_responses_api_response(step.text.as_deref()).or_value_error()?;
            let mock = serde_json::to_value(&response).or_value_error()?;
            let file_search: Vec<Value> = step
                .tools
                .iter()
                .map(|tools| Value::Bool(has_file_search_tool(tools.as_deref())))
                .collect();
            steps.push(json!({"mock": mock, "file_search": file_search}));
        }
        Ok(json!({"steps": steps}))
    }
}
