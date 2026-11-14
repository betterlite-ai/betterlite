pub mod error;
pub mod reply;
pub mod request;
mod tests;

use betterlite_config::loader::EnvironmentLookup;
use betterlite_openai::error::{ProviderError, ProviderErrorParts};
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::credentials::ServerCredential;
use betterlite_openai::search::provider::{
    OutboundSearch, ProviderResponse, SearchProvider, SearchQuery,
};
use betterlite_openai::search::records::SearchResponse;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use error::NimbleSearchError;
use reply::decode_nimble_reply;
use request::{SearchBody, search_body};

pub const NIMBLE_API_BASE: &str = "https://sdk.nimbleway.com/v2";
const NIMBLE_DOCS_URL: &str = "https://docs.nimbleway.com/api-reference/search/search";
const NIMBLE_API_KEY_VAR: &str = "NIMBLE_API_KEY";
const NIMBLE_API_BASE_VAR: &str = "NIMBLE_API_BASE";
const NIMBLE_CREDENTIAL: ServerCredential = ServerCredential {
    key_vars: &[NIMBLE_API_KEY_VAR],
    base_var: Some(NIMBLE_API_BASE_VAR),
    default_base: Some(NIMBLE_API_BASE),
};

#[derive(Debug, Clone)]
pub struct NimbleSearch {
    pub api_key: Option<SecretString>,
    pub api_base: String,
}

impl NimbleSearch {
    #[must_use]
    pub fn load(lookup: &dyn EnvironmentLookup) -> Self {
        let present = |name: &str| lookup.lookup(name).filter(|value| !value.is_empty());
        Self {
            api_key: present(NIMBLE_API_KEY_VAR).map(SecretString::from),
            api_base: present(NIMBLE_API_BASE_VAR).unwrap_or_else(|| NIMBLE_API_BASE.to_owned()),
        }
    }

    #[must_use]
    pub fn endpoint(&self, api_base: Option<&str>) -> String {
        let trimmed = api_base.unwrap_or(&self.api_base).trim_end_matches('/');
        if trimmed.ends_with("/search") {
            trimmed.to_owned()
        } else {
            format!("{trimmed}/search")
        }
    }

    pub fn prepare_headers(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
    ) -> Result<Headers, NimbleSearchError> {
        let environment = NimbleEnvironment {
            api_key: self.api_key.as_ref(),
            api_base: &self.api_base,
        };
        let Some(resolved) = NIMBLE_CREDENTIAL.resolve(api_key.cloned(), api_base, &environment)?
        else {
            return Err(NimbleSearchError::MissingApiKey);
        };
        Ok(auth_headers(headers, &resolved))
    }
}

fn auth_headers(mut headers: Headers, key: &SecretString) -> Headers {
    headers.insert(
        "Authorization".to_owned(),
        format!("Bearer {}", key.expose_secret()),
    );
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    headers.insert("X-Client-Source".to_owned(), "litellm".to_owned());
    headers
}

struct NimbleEnvironment<'settings> {
    api_key: Option<&'settings SecretString>,
    api_base: &'settings str,
}

impl EnvironmentLookup for NimbleEnvironment<'_> {
    fn lookup(&self, name: &str) -> Option<String> {
        match name {
            "NIMBLE_API_KEY" => self.api_key.map(|key| key.expose_secret().to_owned()),
            "NIMBLE_API_BASE" => Some(self.api_base.to_owned()),
            _ => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    detail: Option<String>,
    #[serde(default)]
    message: Option<String>,
}

// Nimble reports errors as `{"detail": ...}` (validation) or `{"success": "false", "message": ...}` (collection)
fn error_detail(message: &str) -> String {
    let Ok(envelope) = serde_json::from_str::<ErrorEnvelope>(message) else {
        return message.to_owned();
    };
    envelope
        .detail
        .filter(|detail| !detail.is_empty())
        .or_else(|| envelope.message.filter(|message| !message.is_empty()))
        .unwrap_or_else(|| message.to_owned())
}

impl SearchProvider for NimbleSearch {
    type Body = SearchBody;
    type Error = NimbleSearchError;

    fn ui_friendly_name() -> &'static str {
        "Nimble"
    }

    fn validate_environment(
        &self,
        headers: Headers,
        api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        let key = api_key
            .filter(|candidate| !candidate.expose_secret().is_empty())
            .or(self.api_key.as_ref());
        match key {
            Some(present) => auth_headers(headers, present),
            None => headers,
        }
    }

    // LiteLLM raises the credential and host checks from validate_environment, which cannot fail here
    fn sign_request(
        &self,
        headers: Headers,
        request: &OutboundSearch<'_, Self::Body>,
    ) -> Result<(Headers, Option<Vec<u8>>), NimbleSearchError> {
        let signed = self.prepare_headers(headers, request.api_key, Some(request.api_base))?;
        Ok((signed, None))
    }

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchBody>,
    ) -> Result<String, NimbleSearchError> {
        Ok(self.endpoint(api_base))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        optional_params: &Object,
    ) -> Result<SearchBody, NimbleSearchError> {
        Ok(search_body(query, optional_params))
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, NimbleSearchError> {
        decode_nimble_reply(response.text.as_bytes()).map_err(|error| {
            let message =
                format!("response does not match the documented /v2/search schema: {error}");
            NimbleSearchError::Provider(Box::new(self.error_class(
                &message,
                response.status_code,
                response.headers,
            )))
        })
    }

    fn error_class(&self, message: &str, status_code: u16, headers: &Headers) -> ProviderError {
        let detail = error_detail(message);
        ProviderError::from(ProviderErrorParts {
            status_code,
            message: format!(
                "Nimble Search: {}. See {NIMBLE_DOCS_URL} for details.",
                detail.trim_end_matches(['.', ' '])
            ),
            headers: Some(headers.clone().into_iter().collect()),
            request: None,
            response: None,
            body: None,
            status_code_is_synthesized: false,
        })
    }
}
