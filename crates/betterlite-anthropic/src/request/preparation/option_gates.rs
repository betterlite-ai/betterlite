use std::error::Error as StdError;

use betterlite_models::decisions::Json;
use betterlite_models::lemonade_model_metadata::ModelInfoClient;
use betterlite_models::model_capability_predicates::CapabilityEnvironment;
use betterlite_models::model_metadata_resolution::dynamic::DynamicMetadataSource;

use crate::anthropic_capability_catalog::sampling::{
    SamplingParam, SamplingRequest, apply_sampling_param,
};
use crate::anthropic_capability_catalog::thinking::maybe_drop_disabled_thinking;
use crate::anthropic_capability_catalog::{CatalogSources, OptionalParams};
use crate::capabilities::speed::{SpeedDecision, SpeedParam, SpeedRequest, maybe_drop_speed_param};

use super::super::error::RequestError;
use super::Preparation;

// The option gates, defaults and cleanup run at their transform_request positions
impl<FileFailure> Preparation<'_, FileFailure>
where
    FileFailure: StdError + Send + Sync + 'static,
{
    pub(super) fn drop_unsupported_speed(
        &mut self,
        catalogs: &CatalogSources<'_>,
    ) -> Result<(), RequestError<FileFailure>> {
        let speed = match self.optional_params.get("speed") {
            Some(value) => SpeedParam::Present(value),
            None => SpeedParam::Absent,
        };
        let request = SpeedRequest {
            model: self.model,
            speed,
            drop: self.drop,
            custom_llm_provider: self.custom_llm_provider,
        };
        if maybe_drop_speed_param(&request, catalogs)? == SpeedDecision::Drop {
            self.optional_params.shift_remove("speed");
        }
        Ok(())
    }

    pub(super) async fn drop_disabled_thinking<Source, Transport>(
        &mut self,
        environment: &mut CapabilityEnvironment<'_, Source, Transport>,
        catalogs: &CatalogSources<'_>,
    ) where
        Source: DynamicMetadataSource,
        Transport: ModelInfoClient,
    {
        let thinking = self
            .optional_params
            .get("thinking")
            .filter(|value| !matches!(value, Json::Null))
            .cloned();
        let mut params = OptionalParams {
            reasoning_effort: None,
            thinking,
            output_config: None,
        };
        let provider = self.provider.as_str();
        if maybe_drop_disabled_thinking(self.model, &mut params, provider, environment, catalogs)
            .await
        {
            self.optional_params.shift_remove("thinking");
        }
    }

    pub(super) fn load_config_defaults(&mut self) -> Result<(), RequestError<FileFailure>> {
        let config = self.defaults.get_config(
            Some(self.model),
            self.catalog,
            self.default_chat_max_tokens,
        )?;
        let Json::Object(fields) = config else {
            return Ok(());
        };
        for (key, value) in fields {
            self.optional_params.entry(key).or_insert(value);
        }
        Ok(())
    }

    pub(super) fn drop_internal_options(&mut self) {
        self.optional_params.shift_remove("is_vertex_request");
        self.optional_params.shift_remove("client_metadata");
    }

    pub(super) fn apply_top_k(
        &mut self,
        catalogs: &CatalogSources<'_>,
    ) -> Result<(), RequestError<FileFailure>> {
        let Some(top_k) = self.optional_params.shift_remove("top_k") else {
            return Ok(());
        };
        if matches!(top_k, Json::Null) {
            return Ok(());
        }
        let request = SamplingRequest {
            model: self.model,
            param: SamplingParam::TopK,
            value: &top_k,
            drop: self.drop,
        };
        if let Some(kept) = apply_sampling_param(&request, catalogs)? {
            self.optional_params.insert("top_k".to_owned(), kept);
        }
        Ok(())
    }
}
