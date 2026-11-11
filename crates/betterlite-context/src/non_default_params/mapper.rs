use betterlite_models::controls::json_value::JsonValue;

use super::Params;
use super::error::MapperError;

pub trait OptionalParamsMapper {
    fn supported_openai_params(&self, model: &str) -> Vec<String>;

    fn map_openai_params(
        &self,
        non_default_params: &Params,
        optional_params: Params,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, MapperError>;

    fn map_response_format(&self, _response_format: &mut JsonValue) {}
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropPolicy {
    pub drop_params: Option<bool>,
    pub litellm_drop_params: bool,
}
