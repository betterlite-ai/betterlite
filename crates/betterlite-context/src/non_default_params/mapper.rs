use betterlite_models::controls::json_value::JsonValue;

use super::Params;
use super::error::MapperError;

// Provider crates sit above betterlite-context, so they implement this where LiteLLM calls a provider config object.
pub trait OptionalParamsMapper {
    fn supported_openai_params(&self, model: &str) -> Vec<String>;

    fn map_openai_params(
        &self,
        non_default_params: &Params,
        optional_params: Params,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, MapperError>;

    // LiteLLM's get_json_schema_from_pydantic_object; most configs leave the response_format as it came.
    fn map_response_format(&self, _response_format: &mut JsonValue) {}
}

// LiteLLM drops an unsupported param when the call's drop_params or the global litellm.drop_params is true.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropPolicy {
    pub drop_params: Option<bool>,
    pub litellm_drop_params: bool,
}
