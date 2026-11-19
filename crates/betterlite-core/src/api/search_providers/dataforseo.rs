pub mod reply;

mod request;
mod tests;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use betterlite_config::env_integer::get_env_int;
use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::{CredentialError, ServerCredential};
use betterlite_openai::search::provider::{
    OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Serialize;
use thiserror::Error;

pub const DATAFORSEO_API_BASE: &str =
    "https://api.dataforseo.com/v3/serp/google/organic/live/advanced";
const LOGIN_VARIABLE: &str = "DATAFORSEO_LOGIN";
const PASSWORD_VARIABLE: &str = "DATAFORSEO_PASSWORD";
const API_BASE_VARIABLE: &str = "DATAFORSEO_API_BASE";
const LOCATION_CODE_VARIABLE: &str = "DEFAULT_DATAFORSEO_LOCATION_CODE";
const DEFAULT_LOCATION_CODE: i64 = 2250;
const AUTHORIZATION_HEADER: &str = "Authorization";
const CONTENT_TYPE_HEADER: &str = "Content-Type";
const JSON_CONTENT_TYPE: &str = "application/json";

const CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[LOGIN_VARIABLE],
    base_var: Some(API_BASE_VARIABLE),
    default_base: Some(DATAFORSEO_API_BASE),
};

#[derive(Debug, Error)]
pub enum DataForSeoSearchError {
    #[error(
        "DATAFORSEO_LOGIN is not set. Set `DATAFORSEO_LOGIN` environment variable or pass credentials in api_key parameter."
    )]
    MissingLogin,
    #[error(
        "DATAFORSEO_PASSWORD is not set. Set `DATAFORSEO_PASSWORD` environment variable or pass credentials in api_key parameter."
    )]
    MissingPassword,
    #[error(transparent)]
    Credential(#[from] CredentialError),
    #[error("the DataForSEO search option `{field}` is not an integer")]
    OptionInteger { field: &'static str },
    #[error("the DataForSEO search option `{field}` is not a string")]
    OptionText { field: &'static str },
    #[error("the DataForSEO search option `{field}` is not a list of strings")]
    OptionList { field: &'static str },
    #[error("the DataForSEO search reply is larger than {limit} bytes")]
    ReplyTooLarge { limit: usize },
    #[error("the DataForSEO search reply does not match the DataForSEO schema")]
    ReplyDecode(#[source] serde_json::Error),
    #[error("the DataForSEO search reply field `tasks` is null")]
    TasksShape,
    #[error("the DataForSEO search reply field `result` is null")]
    ResultShape,
    #[error("the DataForSEO search reply field `items` is null")]
    ItemsShape,
    #[error("the DataForSEO search reply result field `{field}` is null")]
    ResultField { field: &'static str },
    #[error("the DataForSEO search reply result field `{field}` has the wrong type")]
    ReplyField {
        field: &'static str,
        #[source]
        source: serde_json::Error,
    },
    #[error("the DataForSEO search reply holds a value that is not a JSON object")]
    NotAnObject,
}

#[derive(Debug, Clone)]
pub struct DataForSeoSearchSettings {
    pub login: Option<SecretString>,
    pub password: Option<SecretString>,
    pub api_base: String,
    pub location_code: i64,
}

impl DataForSeoSearchSettings {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            login: present(LOGIN_VARIABLE).map(SecretString::from),
            password: present(PASSWORD_VARIABLE).map(SecretString::from),
            api_base: present(API_BASE_VARIABLE).unwrap_or_else(|| DATAFORSEO_API_BASE.to_owned()),
            location_code: get_env_int(
                lookup.lookup(LOCATION_CODE_VARIABLE).as_deref(),
                DEFAULT_LOCATION_CODE,
            ),
        }
    }
}

// LiteLLM guards a complete login/password pair only, so the guard sees the login only beside a password
impl EnvironmentLookup for DataForSeoSearchSettings {
    fn lookup(&self, name: &str) -> Option<String> {
        if name == API_BASE_VARIABLE {
            return Some(self.api_base.clone());
        }
        if name != LOGIN_VARIABLE || self.password.is_none() {
            return None;
        }
        self.login
            .as_ref()
            .map(|login| login.expose_secret().to_owned())
    }
}

// the task object DataForSEO reads from the one-element request array
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DataForSeoSearchTask {
    pub keyword: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depth: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<Vec<String>>,
    pub language_code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location_code: Option<i64>,
    // DataForSEO's native parameters, which betterlite copies as given
    #[serde(flatten)]
    pub extra: Object,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct DataForSeoSearchRequest(pub Vec<DataForSeoSearchTask>);

#[derive(Debug, Clone)]
pub struct DataForSeoSearchConfig {
    pub settings: DataForSeoSearchSettings,
}

impl DataForSeoSearchConfig {
    pub fn validated_headers(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, DataForSeoSearchError> {
        let (login, password) = self.credentials(api_key, api_base)?;
        let encoded = STANDARD.encode(format!("{login}:{password}"));
        headers.insert(AUTHORIZATION_HEADER.to_owned(), format!("Basic {encoded}"));
        headers.insert(CONTENT_TYPE_HEADER.to_owned(), JSON_CONTENT_TYPE.to_owned());
        Ok(headers)
    }

    // a caller key without a colon is ignored, and one with a colon replaces both halves by splitting once
    fn credentials(
        &self,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<(String, String), DataForSeoSearchError> {
        if let Some(key) = api_key.map(SecretString::expose_secret)
            && let Some((login, password)) = key.split_once(':')
        {
            return checked_pair(login, password);
        }
        let resolved = CREDENTIAL.resolve(None, api_base, &self.settings)?;
        let login = match resolved {
            Some(login) => login.expose_secret().to_owned(),
            None => self
                .settings
                .login
                .as_ref()
                .ok_or(DataForSeoSearchError::MissingLogin)?
                .expose_secret()
                .to_owned(),
        };
        let password = self
            .settings
            .password
            .as_ref()
            .ok_or(DataForSeoSearchError::MissingPassword)?
            .expose_secret()
            .to_owned();
        Ok((login, password))
    }
}

fn checked_pair(login: &str, password: &str) -> Result<(String, String), DataForSeoSearchError> {
    if login.is_empty() {
        return Err(DataForSeoSearchError::MissingLogin);
    }
    if password.is_empty() {
        return Err(DataForSeoSearchError::MissingPassword);
    }
    Ok((login.to_owned(), password.to_owned()))
}

impl SearchProvider for DataForSeoSearchConfig {
    type Body = DataForSeoSearchRequest;
    type Error = DataForSeoSearchError;

    fn ui_friendly_name() -> &'static str {
        "DataForSEO"
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

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&Self::Body>,
    ) -> Result<String, Self::Error> {
        let base = api_base
            .filter(|base| !base.is_empty())
            .unwrap_or(&self.settings.api_base);
        Ok(base.to_owned())
    }

    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), Self::Error> {
        let validated = self.validated_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((validated, None))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<Self::Body, Self::Error> {
        request::build(query, optional_params, self.settings.location_code)
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Self::Error> {
        reply::decode_dataforseo_search(response.text.as_bytes())
    }
}
