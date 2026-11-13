use betterlite_context::non_default_params::Params;

use super::error::ResponsesParametersError;

pub trait ResponsesParamProvider {
    type Error: From<ResponsesParametersError>;

    fn name(&self) -> &str;

    fn supported_params(&self, model: &str) -> Result<Option<Vec<String>>, Self::Error>;

    fn map_params(
        &self,
        params: &Params,
        model: &str,
        drop_params: bool,
    ) -> Result<Params, Self::Error>;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DropPolicy {
    pub global: bool,
    pub call: Option<bool>,
}
