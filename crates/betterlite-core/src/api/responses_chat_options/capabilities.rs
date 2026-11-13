use std::convert::Infallible;

use betterlite_config::sections::settings::JsonValue;
use betterlite_models::full_model_metadata::parameters::SupportedParameterSource;

use crate::responses_bridge::{
    BridgeRequest, ModelInfoError, ResponsesBridgeCapabilities, responses_api_bridge_check,
};

const WEB_SEARCH_OPTIONS: &str = "web_search_options";
const MODE: &str = "mode";
const RESPONSES_MODE: &str = "responses";

// LiteLLM wraps the same check in a try/except, so a capability lookup failure only drops the summary
pub trait CompletionBridgeProbe {
    fn bridges_back(&self, request: &BridgeRequest<'_>) -> Result<bool, ModelInfoError>;
}

pub struct SdkBridgeProbe<'capabilities, Capabilities: ResponsesBridgeCapabilities + ?Sized> {
    pub capabilities: &'capabilities Capabilities,
}

impl<Capabilities: ResponsesBridgeCapabilities + ?Sized> CompletionBridgeProbe
    for SdkBridgeProbe<'_, Capabilities>
{
    fn bridges_back(&self, request: &BridgeRequest<'_>) -> Result<bool, ModelInfoError> {
        let check = responses_api_bridge_check(request, self.capabilities);
        Ok(matches!(
            check.model_info.get(MODE),
            Some(JsonValue::String(mode)) if mode == RESPONSES_MODE
        ))
    }
}

// an unmapped provider has no capability list, so LiteLLM keeps the derived options
pub(crate) fn drops_web_search_options(
    model: &str,
    custom_llm_provider: Option<&str>,
    parameters: &dyn SupportedParameterSource<Error = Infallible>,
) -> bool {
    let supported = parameters.supported_openai_params(model, custom_llm_provider);
    let listed = match supported {
        Ok(listed) => listed,
        Err(never) => match never {},
    };
    listed.is_some_and(|names| !names.iter().any(|name| name == WEB_SEARCH_OPTIONS))
}
