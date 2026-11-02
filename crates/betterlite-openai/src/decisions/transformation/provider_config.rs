use rust_decimal::Decimal;
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::error::Category;

use betterlite_models::decisions::{DecisionsJson, Extra, ir};

use crate::error::{ProviderError, ProviderErrorParts};
use crate::openai_base_config::Headers;

use super::requests::{self, RequestConversionError};
use super::responses;
use super::wire;

const RESERVED_HEADERS: [&str; 2] = ["authorization", "content-type"];

pub struct SigningContext<'call> {
    pub url: &'call str,
    pub api_base: &'call str,
    pub body: &'call wire::RequestWire,
    pub api_key: Option<&'call SecretString>,
    pub litellm_params: &'call Extra,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseResponseError {
    #[error("{0}")]
    Malformed(#[source] serde_json::Error),
    #[error("{}", .0.message)]
    Invalid(Box<ProviderError>),
}

fn response_error(custom_llm_provider: &str, source: &serde_json::Error) -> ProviderError {
    ProviderError::from(ProviderErrorParts {
        status_code: 500,
        message: format!(
            "Decisions provider '{custom_llm_provider}' returned an unexpected response: {source}"
        ),
        headers: None,
        request: None,
        response: None,
        body: None,
        status_code_is_synthesized: false,
    })
}

#[must_use]
fn first_secret(names: &[&str], get_secret_str: fn(&str) -> String) -> Option<String> {
    names
        .iter()
        .map(|name| get_secret_str(name))
        .find(|value| !value.is_empty())
}

fn resolve_credential(
    explicit: Option<&str>,
    env_names: &[&str],
    get_secret_str: fn(&str) -> String,
    default: Option<String>,
) -> Option<String> {
    explicit
        .map(str::to_owned)
        .or_else(|| first_secret(env_names, get_secret_str))
        .or(default)
}

pub trait DecisionsProviderConfig {
    const PATH: &'static str = "/v1/systemone";
    const API_KEY_ENV: &'static [&'static str] = &[];
    const API_BASE_ENV: &'static [&'static str] = &[];
    const API_KEY_REQUIRED: bool = true;
    const SUPPORTS_SAFETY_IDENTIFIER: bool = false;

    fn get_default_api_base(&self) -> Option<String> {
        None
    }

    fn provider_reported_cost(&self, _response: &ir::DecisionResponse) -> Option<Decimal> {
        None
    }

    fn missing_api_base_message(&self, custom_llm_provider: &str) -> String {
        format!("api_base is required for Decisions provider '{custom_llm_provider}'")
    }

    fn resolve_api_base(
        &self,
        api_base: Option<&str>,
        get_secret_str: fn(&str) -> String,
    ) -> Option<String> {
        resolve_credential(
            api_base,
            Self::API_BASE_ENV,
            get_secret_str,
            self.get_default_api_base(),
        )
    }

    fn resolve_api_key(
        &self,
        api_key: Option<&str>,
        get_secret_str: fn(&str) -> String,
    ) -> Option<String> {
        resolve_credential(api_key, Self::API_KEY_ENV, get_secret_str, None)
    }

    fn canonical_model(&self, model: &str) -> String {
        model.to_owned()
    }

    fn request_model(&self, model: &str) -> String {
        model.to_owned()
    }

    fn validate_environment(
        &self,
        headers: &Headers,
        _model: &str,
        api_key: Option<&SecretString>,
    ) -> Headers {
        let mut validated: Headers = headers
            .iter()
            .filter(|(name, _)| !RESERVED_HEADERS.contains(&name.to_lowercase().as_str()))
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        if let Some(key) = api_key {
            validated.insert(
                "Authorization".to_owned(),
                format!("Bearer {}", key.expose_secret()),
            );
        }
        validated.insert("Content-Type".to_owned(), "application/json".to_owned());
        validated
    }

    fn get_complete_url(&self, api_base: &str, _model: &str) -> String {
        let trimmed = api_base.trim_end_matches('/');
        let base = trimmed.strip_suffix("/v1").unwrap_or(trimmed);
        format!("{base}{}", Self::PATH)
    }

    fn signs_with_aws(&self, _api_base: &str) -> bool {
        false
    }

    fn sign_request(
        &self,
        headers: Headers,
        _context: SigningContext<'_>,
    ) -> (Headers, Option<Vec<u8>>) {
        (headers, None)
    }

    fn transform_decisions_request(
        &self,
        model: &str,
        request: &ir::DecisionRequest,
        _custom_llm_provider: &str,
    ) -> Result<wire::RequestWire, RequestConversionError> {
        let built = requests::ir_to_systemone_request(&self.request_model(model), request)?;
        Ok(wire::RequestWire::from(&built))
    }

    fn unwrap_response<'call>(&self, payload: &'call [u8]) -> &'call [u8] {
        payload
    }

    fn parse_response(
        &self,
        payload: &[u8],
        request: &ir::DecisionRequest,
    ) -> Result<ir::DecisionResponse, serde_json::Error> {
        responses::to_ir::parse_systemone_response(self.unwrap_response(payload), request)
    }

    fn transform_decisions_response(
        &self,
        _model: &str,
        custom_llm_provider: &str,
        raw_response: &[u8],
        request: &ir::DecisionRequest,
    ) -> Result<ir::DecisionResponse, ParseResponseError> {
        self.parse_response(raw_response, request)
            .map_err(|source| {
                if source.classify() == Category::Data {
                    ParseResponseError::Invalid(Box::new(response_error(
                        custom_llm_provider,
                        &source,
                    )))
                } else {
                    ParseResponseError::Malformed(source)
                }
            })
    }

    fn get_error_class(
        &self,
        error_message: &str,
        status_code: u16,
        headers: &Headers,
    ) -> ProviderError {
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: error_message.to_owned(),
            headers: Some(headers.clone().into_iter().collect()),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }

    fn health_check_questions(&self) -> Vec<ir::DecisionQuestion> {
        vec![ir::DecisionQuestion::Predicate(ir::PredicateQuestion {
            name: Some("reachable".to_owned()),
            instructions: Some(DecisionsJson::Text("Is the service reachable?".to_owned())),
            criteria: None,
            extra: Extra::default(),
        })]
    }
}
