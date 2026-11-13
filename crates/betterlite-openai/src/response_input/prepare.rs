use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;

use super::error::ResponseInputError;
use super::input::{validate_input, without_foreign_item_ids};
use super::tools::{sanitized_tool_schemas, tools_with_object_parameters, without_cache_control};
use super::{ResponsesInput, ResponsesTool};

pub(crate) fn prepared_input_and_tools(
    model: &str,
    input: &ResponsesInput,
    tools: Option<Vec<ResponsesTool>>,
    provider: ProviderId,
    model_info: Option<&Object>,
) -> Result<(ResponsesInput, Option<Vec<ResponsesTool>>), ResponseInputError> {
    let (stripped_input, stripped_tools) = without_cache_control(validate_input(input), tools);
    let decoded_tools = tools_with_object_parameters(model, provider, stripped_tools)?;
    Ok((
        without_foreign_item_ids(provider, stripped_input),
        sanitized_tool_schemas(model, provider, decoded_tools, model_info),
    ))
}
