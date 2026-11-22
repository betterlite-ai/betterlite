use betterlite_models::decisions::ir;
use betterlite_openai::decisions::call::provider::ProviderConfig;
use betterlite_openai::decisions::transformation::provider_config::{
    DecisionsProviderConfig, ParseResponseError, SigningContext,
};
use betterlite_openai::error::ProviderError;
use betterlite_openai::openai_base_config::Headers;
use rust_decimal::Decimal;
use secrecy::SecretString;

// The BaseDecisionsConfig members the send stage calls. The provider trait carries associated
// constants, so it cannot be made into an object; this view can.
pub trait DecisionsSendConfig: Send + Sync {
    fn validate_environment(
        &self,
        headers: &Headers,
        model: &str,
        api_key: Option<&SecretString>,
    ) -> Headers;

    fn get_complete_url(&self, api_base: &str, model: &str) -> String;

    fn signs_with_aws(&self, api_base: &str) -> bool;

    fn sign_request(
        &self,
        headers: Headers,
        context: SigningContext<'_>,
    ) -> (Headers, Option<Vec<u8>>);

    fn transform_decisions_response(
        &self,
        model: &str,
        custom_llm_provider: &str,
        raw_response: &[u8],
        request: &ir::DecisionRequest,
    ) -> Result<ir::DecisionResponse, ParseResponseError>;

    fn provider_reported_cost(&self, response: &ir::DecisionResponse) -> Option<Decimal>;

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError;
}

impl<T: DecisionsProviderConfig + Send + Sync> DecisionsSendConfig for T {
    fn validate_environment(
        &self,
        headers: &Headers,
        model: &str,
        api_key: Option<&SecretString>,
    ) -> Headers {
        DecisionsProviderConfig::validate_environment(self, headers, model, api_key)
    }

    fn get_complete_url(&self, api_base: &str, model: &str) -> String {
        DecisionsProviderConfig::get_complete_url(self, api_base, model)
    }

    fn signs_with_aws(&self, api_base: &str) -> bool {
        DecisionsProviderConfig::signs_with_aws(self, api_base)
    }

    fn sign_request(
        &self,
        headers: Headers,
        context: SigningContext<'_>,
    ) -> (Headers, Option<Vec<u8>>) {
        DecisionsProviderConfig::sign_request(self, headers, context)
    }

    fn transform_decisions_response(
        &self,
        model: &str,
        custom_llm_provider: &str,
        raw_response: &[u8],
        request: &ir::DecisionRequest,
    ) -> Result<ir::DecisionResponse, ParseResponseError> {
        DecisionsProviderConfig::transform_decisions_response(
            self,
            model,
            custom_llm_provider,
            raw_response,
            request,
        )
    }

    fn provider_reported_cost(&self, response: &ir::DecisionResponse) -> Option<Decimal> {
        DecisionsProviderConfig::provider_reported_cost(self, response)
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        DecisionsProviderConfig::get_error_class(self, error_message, status_code, headers)
    }
}

// ProviderConfigManager.get_provider_decisions_config returns one of eight boxed configs
pub(crate) fn of(provider: &ProviderConfig) -> &dyn DecisionsSendConfig {
    match provider {
        ProviderConfig::Openai(config) => config.as_ref(),
        ProviderConfig::Openrouter(config) => config.as_ref(),
        ProviderConfig::AzureAi(config) => config.as_ref(),
        ProviderConfig::Perplexity(config) => config.as_ref(),
        ProviderConfig::Typesafe(config) => config.as_ref(),
        ProviderConfig::StrandsDecider(config) => config.as_ref(),
        ProviderConfig::Cloudflare(config) => config.as_ref(),
        ProviderConfig::HostedVllm(config) => config.as_ref(),
    }
}
