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
}

// LiteLLM drops an unsupported param when the call's drop_params or the global litellm.drop_params is true.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropPolicy {
    pub drop_params: Option<bool>,
    pub litellm_drop_params: bool,
}
