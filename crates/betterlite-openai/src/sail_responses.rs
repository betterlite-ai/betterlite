use betterlite_models::identity::provider_id::ProviderId;
use betterlite_prompts::arguments::json::Object;

use crate::json_provider_policy::responses::ResponsesPolicy;
use crate::reasoning_models::DropParams;
use crate::response_input::ResponsesInput;
use crate::response_params;
use crate::response_params::bundled::BundledReasoningModels;
use crate::response_params::capabilities::ResponsesCapabilities;
use crate::response_params::params::{ResponsesOptions, ResponsesParams};
use crate::sail_common::SailSettings;

// ast-grep-ignore: no-long-names
use crate::sail_common::responses::responses_params_with_completion_window as window_params;

pub mod error;
mod tests;

use self::error::SailResponsesError;

const PROVIDER: ProviderId = ProviderId::Sail;
const SERVICE_TIER: &str = "service_tier";

#[derive(Debug, Clone)]
pub struct SailResponsesConfig {
    pub policy: ResponsesPolicy,
    pub settings: SailSettings,
}

pub struct MapRequest<'request, C> {
    pub capabilities: &'request mut C,
    pub model: &'request str,
    pub params: &'request ResponsesParams,
    pub drop_params: DropParams,
    pub bundled: &'request mut BundledReasoningModels,
}

impl SailResponsesConfig {
    pub async fn map_openai_params<C>(
        &self,
        request: MapRequest<'_, C>,
    ) -> Result<Object, SailResponsesError>
    where
        C: ResponsesCapabilities,
    {
        let MapRequest {
            capabilities,
            model,
            params,
            drop_params,
            bundled,
        } = request;
        let options = ResponsesOptions {
            provider: PROVIDER,
            model: model.to_owned(),
            params: params.clone(),
        };
        let mapped =
            response_params::map_openai_params(capabilities, &options, drop_params, bundled)
                .await?;
        let raw = serde_json::to_value(&mapped)
            .map_err(|source| SailResponsesError::Encode { source })?;
        let object: Object =
            serde_json::from_value(raw).map_err(|source| SailResponsesError::Encode { source })?;
        // LiteLLM's one drop flag is litellm.drop_params or the call's argument, whichever is set
        let dropping = drop_params.global || drop_params.request;
        Ok(window_params(&object, model, dropping, &self.settings)?)
    }

    pub fn transform_request(
        &self,
        model: &str,
        input: &ResponsesInput,
        params: &Object,
        model_info: Option<&Object>,
    ) -> Result<Object, SailResponsesError> {
        let mut body = self
            .policy
            .transform_request(model, input, params, model_info)?;
        body.shift_remove(SERVICE_TIER);
        Ok(body)
    }
}
