pub mod reply;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_config::sections::Sections;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use indexmap::IndexMap;
use secrecy::{ExposeSecret as _, SecretString};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::api::responses_input_content::is_truthy;

use super::google_pse::wire::{self, SearchParam, WireError};

mod params;
mod request;
mod tests;

pub const APISERPENT_API_BASE: &str = "https://apiserpent.com";
const API_KEY_VARIABLE: &str = "APISERPENT_API_KEY";
const API_BASE_VARIABLE: &str = "APISERPENT_API_BASE";
const API_KEY_HEADER: &str = "X-API-Key";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";
const QUICK_SEARCH_PATH: &str = "/api/search/quick";
const DEEP_SEARCH_PATH: &str = "/api/search";

const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[API_KEY_VARIABLE],
    base_var: Some(API_BASE_VARIABLE),
    default_base: Some(APISERPENT_API_BASE),
};

#[derive(Debug, Error)]
pub enum ApiserpentSearchError {
    #[error("APISERPENT_API_KEY is not set. Set `APISERPENT_API_KEY` environment variable.")]
    MissingApiKey,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the APISerpent search option `{field}` has the wrong type")]
    OptionType { field: &'static str },
    #[error("the APISerpent search option `{field}` is not a list of strings")]
    OptionList { field: &'static str },
    #[error("num must be between 1 and 100, got {value}")]
    NumRange { value: i64 },
    #[error("pages must be between 1 and 10, got {value}")]
    PagesRange { value: i64 },
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("the APISerpent search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the APISerpent search reply does not match the APISerpent schema")]
    ReplyDecode(#[source] serde_json::Error),
    #[error("the APISerpent search reply field `results` is neither a list nor an object")]
    ResultsShape,
    #[error("the APISerpent search reply field `organic` is not a list of results")]
    OrganicShape,
    #[error("the APISerpent search reply holds a value that is not a JSON object")]
    NotAnObject,
}

#[derive(Debug, Clone)]
pub struct ApiserpentSearchConfig {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl ApiserpentSearchConfig {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let (api_key, api_base) = Sections::apiserpent_search_settings(lookup);
        Self { api_key, api_base }
    }

    pub fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, ApiserpentSearchError> {
        let environment = ApiserpentEnvironment {
            api_key: self.api_key.as_ref(),
            api_base: &self.api_base,
        };
        let Some(resolved) = CREDENTIAL.resolve(api_key.cloned(), api_base, &environment)? else {
            return Err(ApiserpentSearchError::MissingApiKey);
        };
        headers.insert(
            API_KEY_HEADER.to_owned(),
            resolved.expose_secret().to_owned(),
        );
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        Ok(headers)
    }
}

struct ApiserpentEnvironment<'settings> {
    api_key: Option<&'settings SecretString>,
    api_base: &'settings str,
}

impl EnvironmentLookup for ApiserpentEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "APISERPENT_API_KEY" => self.api_key.map(|key| key.expose_secret().to_owned()),
            "APISERPENT_API_BASE" => Some(self.api_base.to_owned()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiserpentBody {
    #[serde(rename = "_apiserpent_params")]
    pub params: IndexMap<String, SearchParam>,
}

impl SearchProvider for ApiserpentSearchConfig {
    type Body = ApiserpentBody;
    type Error = ApiserpentSearchError;

    fn ui_friendly_name() -> &'static str {
        "APISerpent"
    }

    fn http_method(&self) -> HttpMethod {
        HttpMethod::Get
    }

    // B3270's hook returns headers only; the same check fails in sign_request before any request leaves
    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Headers {
        self.validated_headers(headers.clone(), api_key, api_base)
            .unwrap_or(headers)
    }

    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        let validated = self.validated_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((validated, None))
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        optional_params: &Object,
        body: Option<&Self::Body>,
    ) -> Result<String, Self::Error> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(&self.api_base)
            .trim_end_matches('/');
        let path = if is_truthy(optional_params.get(request::DEEP_PARAM)) {
            DEEP_SEARCH_PATH
        } else {
            QUICK_SEARCH_PATH
        };
        let endpoint = if base.ends_with(path) {
            base.to_owned()
        } else {
            format!("{base}{path}")
        };
        match body {
            Some(record) => Ok(format!(
                "{endpoint}?{}",
                wire::query_string(&record.params, true)?
            )),
            None => Ok(endpoint),
        }
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<Self::Body, Self::Error> {
        request::build(query, optional_params)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Self::Error> {
        reply::decode_apiserpent_search(response.text.as_bytes())
    }
}
