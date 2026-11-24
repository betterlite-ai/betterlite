mod input;
mod ops;

use betterlite_mcp::openapi_tools::naming::sanitize_openapi_tool_name;
use betterlite_oracle_helper_api::helper::{Helper, HelperResult};
use serde_json::json;

use input::Input;

pub(crate) struct Op;

impl Helper for Op {
    type Input = Input;

    fn run(input: &Self::Input) -> HelperResult {
        match input {
            Input::ToolName(value) => Ok(json!({"name": sanitize_openapi_tool_name(&value.raw)})),
            Input::BaseUrl(value) => ops::base_url(value),
            Input::ResolveParams(value) => ops::resolve_params(value),
            Input::ExtractParams(value) => ops::extract_params(value),
            Input::InputSchema(value) => ops::input_schema(value),
            Input::MergeHeaders(value) => Ok(ops::merge_headers(value)),
            Input::Call(value) => ops::call(value),
        }
    }
}
