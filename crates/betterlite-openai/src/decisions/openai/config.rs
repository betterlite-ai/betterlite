use secrecy::ExposeSecret as _;

use betterlite_models::decisions::ir;
use betterlite_models::decisions::openai;

use crate::credentials::LiteLlmGlobals;
use crate::decisions::openai::{request, response};
use crate::decisions::transformation::provider_config::{DecisionsProviderConfig, first_secret};
use crate::decisions::transformation::requests::RequestConversionError;
use crate::decisions::transformation::wire::ProviderRequestWire;

const BASE_ENV: [&str; 2] = ["OPENAI_BASE_URL", "OPENAI_API_BASE"];
const KEY_ENV: [&str; 1] = ["OPENAI_API_KEY"];
const DEFAULT_API_BASE: &str = "https://api.openai.com";

#[derive(Debug, Default)]
pub struct OpenAiDecisionsConfig {
    pub globals: LiteLlmGlobals,
}

#[derive(Debug, Clone, Copy)]
pub enum Credential {
    Base,
    Key,
}

impl OpenAiDecisionsConfig {
    #[must_use]
    pub fn resolve(
        &self,
        explicit: Option<&str>,
        credential: Credential,
        get_secret_str: &dyn Fn(&str) -> String,
    ) -> Option<String> {
        if let Some(given) = explicit {
            return Some(given.to_owned());
        }
        match credential {
            Credential::Base => self
                .globals
                .api_base
                .clone()
                .or_else(|| first_secret(&BASE_ENV, get_secret_str))
                .or_else(|| self.get_default_api_base()),
            Credential::Key => {
                let secrets = [
                    self.globals.api_key.as_ref(),
                    self.globals.openai_key.as_ref(),
                ];
                secrets.into_iter().flatten().next().map_or_else(
                    || first_secret(&KEY_ENV, get_secret_str),
                    |secret| Some(secret.expose_secret().to_owned()),
                )
            }
        }
    }
}

impl DecisionsProviderConfig for OpenAiDecisionsConfig {
    const PATH: &'static str = "/v1/decisions";
    const SUPPORTS_SAFETY_IDENTIFIER: bool = true;

    fn get_default_api_base(&self) -> Option<String> {
        Some(DEFAULT_API_BASE.to_owned())
    }

    fn resolve_api_base(
        &self,
        api_base: Option<&str>,
        get_secret_str: fn(&str) -> String,
    ) -> Option<String> {
        self.resolve(api_base, Credential::Base, &get_secret_str)
    }

    fn resolve_api_key(
        &self,
        api_key: Option<&str>,
        get_secret_str: fn(&str) -> String,
    ) -> Option<String> {
        self.resolve(api_key, Credential::Key, &get_secret_str)
    }

    fn transform_decisions_request(
        &self,
        model: &str,
        request: &ir::DecisionRequest,
        _custom_llm_provider: &str,
    ) -> Result<ProviderRequestWire, RequestConversionError> {
        let wire = request::ir_to_openai_request(&self.request_model(model), request)?;
        Ok(ProviderRequestWire::OpenAi(wire))
    }

    fn parse_response(
        &self,
        payload: &[u8],
        request: &ir::DecisionRequest,
    ) -> Result<ir::DecisionResponse, serde_json::Error> {
        let response: openai::DecisionResponse = serde_json::from_slice(payload)?;
        Ok(response::to_ir::openai_response_to_ir(&response, request))
    }
}
