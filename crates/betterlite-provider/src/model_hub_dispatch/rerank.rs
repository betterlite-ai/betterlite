use super::HubDispatchError;
use super::credentials::{
    Globals, HUGGINGFACE_RERANK_BASE, HUGGINGFACE_RERANK_KEY, Sources, huggingface_env, resolve,
};
use super::tokens::Cl100k;
use crate::api::rerank_contracts::{
    BaseRerankConfig, EnvironmentRequest, MapCohereRerankParams, Params, ResponseTransform,
};
use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::api::rerank_contracts::records::RerankResponse;
use betterlite_openai::huggingface_chat::HuggingfaceEnv;
use betterlite_openai::huggingface_rerank::error::HuggingfaceRerankError;
use betterlite_openai::huggingface_rerank::response::ResponseRequest;
use betterlite_openai::huggingface_rerank::{Uuid4, headers, params, response, url};
use betterlite_openai::openai_base_config::Headers;
use betterlite_prompts::arguments::json::Object;
use secrecy::{ExposeSecret, SecretString};
pub struct HuggingFaceRerankConfig {
    pub key: Option<SecretString>,
    pub api_base: Option<String>,
    pub env: HuggingfaceEnv,
    pub counter: Cl100k,
}
impl BaseRerankConfig for HuggingFaceRerankConfig {
    type Error = HuggingfaceRerankError;
    fn validate_environment(
        &self,
        request: EnvironmentRequest<'_>,
    ) -> Result<Headers, Self::Error> {
        let key = request.api_key.or(self.key.as_ref());
        Ok(headers::validate_environment(request.headers, key))
    }
    fn transform_rerank_request(
        &self,
        _model: &str,
        optional_rerank_params: &Params,
        _headers: &Headers,
        _litellm_params: Option<&Params>,
    ) -> Result<Params, Self::Error> {
        params::transform_rerank_request(optional_rerank_params)
    }
    fn transform_rerank_response(
        &self,
        _model_response: RerankResponse,
        request: ResponseTransform<'_>,
    ) -> Result<RerankResponse, Self::Error> {
        let raw = request.raw_response;
        response::transform_rerank_response(
            &ResponseRequest {
                model: request.model,
                status_code: Some(raw.status_code),
                body: &raw.body,
                request_data: request.request_data,
            },
            &self.counter,
            &Uuid4,
        )
    }
    fn get_complete_url(
        &self,
        api_base: Option<&str>,
        _model: &str,
        _optional_params: Option<&Params>,
    ) -> String {
        url::get_complete_url(api_base.or(self.api_base.as_deref()), &self.env)
    }
    fn supported_rerank_params(&self, _model: &str) -> Vec<String> {
        params::SUPPORTED_RERANK_PARAMS
            .iter()
            .map(|name| (*name).to_owned())
            .collect()
    }
    fn map_cohere_rerank_params(
        &self,
        request: MapCohereRerankParams<'_>,
    ) -> Result<Params, Self::Error> {
        Ok(params::map_cohere_rerank_params(Some(
            request.non_default_params,
        )))
    }
}
pub fn huggingface_rerank_config(
    api_key: Option<&SecretString>,
    api_base: Option<&str>,
    globals: &Globals<'_>,
    env: &(dyn EnvironmentLookup + Sync),
) -> Result<HuggingFaceRerankConfig, HubDispatchError> {
    let keys = Sources {
        call: api_key.map(ExposeSecret::expose_secret),
        globals,
        env,
    };
    let bases = Sources {
        call: api_base,
        globals,
        env,
    };
    let base = resolve(HUGGINGFACE_RERANK_BASE, &bases, &mut Object::new())?;
    Ok(HuggingFaceRerankConfig {
        key: resolve(HUGGINGFACE_RERANK_KEY, &keys, &mut Object::new())?,
        api_base: base.map(|value| value.expose_secret().to_owned()),
        env: huggingface_env(env),
        counter: Cl100k::new()?,
    })
}
