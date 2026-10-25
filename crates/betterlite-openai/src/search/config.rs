use std::collections::BTreeSet;

use thiserror::Error;

pub const PERPLEXITY_UNIFIED_PARAMS: &[&str] = &[
    "max_results",
    "search_domain_filter",
    "country",
    "max_tokens_per_page",
];

pub type SecretLookup<'source> = dyn Fn(&str) -> Option<String> + 'source;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum SearchConfigError {
    #[error("refusing to send the server-configured {credential_name} to the caller-supplied api_base '{api_base}'")]
    UntrustedCredential {
        credential_name: String,
        api_base: String,
    },
    #[error("{0} must be implemented by provider")]
    NotImplemented(&'static str),
}

#[derive(Debug, Clone, Copy)]
pub struct CredentialRequest<'request> {
    pub caller_api_key: Option<&'request str>,
    pub caller_api_base: Option<&'request str>,
    pub key_env_vars: &'request [&'request str],
    pub base_env_var: Option<&'request str>,
    pub default_api_base: Option<&'request str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchError {
    pub status_code: i64,
    pub message: String,
    pub headers: BTreeSet<(String, String)>,
}

pub struct BaseSearchConfig;

impl BaseSearchConfig {
    #[must_use]
    pub const fn ui_friendly_name() -> &'static str {
        "Unknown Search Provider"
    }

    #[must_use]
    pub const fn supports_rich_search_input(&self) -> bool {
        false
    }

    #[must_use]
    pub const fn get_http_method(&self) -> &'static str {
        "POST"
    }

    #[must_use]
    pub const fn get_supported_perplexity_optional_params() -> &'static [&'static str] {
        PERPLEXITY_UNIFIED_PARAMS
    }

    pub fn resolve_server_api_key(
        &self,
        request: &CredentialRequest<'_>,
        lookup: &SecretLookup<'_>,
    ) -> Result<Option<String>, SearchConfigError> {
        if let Some(api_key) = request.caller_api_key {
            return Ok(Some(api_key.to_owned()));
        }
        let resolved_key = request.key_env_vars.iter().find_map(|name| lookup(name));
        let Some(server_key) = resolved_key else {
            return Ok(None);
        };
        if let Some(api_base) = request.caller_api_base {
            let override_base = request.base_env_var.and_then(lookup);
            let trusted = [request.default_api_base, override_base.as_deref()]
                .into_iter()
                .flatten()
                .any(|base| {
                    search_host(api_base) == search_host(base) && !search_host(api_base).is_empty()
                });
            if !trusted {
                let credential_name = request
                    .key_env_vars
                    .first()
                    .copied()
                    .unwrap_or("search credential")
                    .to_owned();
                return Err(SearchConfigError::UntrustedCredential {
                    credential_name,
                    api_base: api_base.to_owned(),
                });
            }
        }
        Ok(Some(server_key))
    }

    #[must_use]
    pub const fn validate_environment(
        &self,
        headers: BTreeSet<(String, String)>,
        _api_key: Option<&str>,
        _api_base: Option<&str>,
    ) -> BTreeSet<(String, String)> {
        headers
    }

    #[must_use]
    pub const fn sign_request(
        &self,
        headers: BTreeSet<(String, String)>,
    ) -> (BTreeSet<(String, String)>, Option<Vec<u8>>) {
        (headers, None)
    }

    pub const fn get_complete_url(&self) -> Result<String, SearchConfigError> {
        Err(SearchConfigError::NotImplemented("get_complete_url"))
    }

    pub const fn transform_search_request(&self) -> Result<(), SearchConfigError> {
        Err(SearchConfigError::NotImplemented("transform_search_request"))
    }

    pub const fn transform_search_response(&self) -> Result<(), SearchConfigError> {
        Err(SearchConfigError::NotImplemented("transform_search_response"))
    }

    #[must_use]
    pub fn get_http_error_class(
        &self,
        status_code: i64,
        message: &str,
        headers: BTreeSet<(String, String)>,
    ) -> SearchError {
        self.get_error_class(message, status_code, headers)
    }

    #[must_use]
    pub fn get_error_class(
        &self,
        message: &str,
        status_code: i64,
        headers: BTreeSet<(String, String)>,
    ) -> SearchError {
        SearchError {
            status_code,
            message: message.to_owned(),
            headers,
        }
    }
}

fn search_host(url: &str) -> String {
    let Some((_, remainder)) = url.split_once("://") else {
        return String::new();
    };
    remainder
        .split(['/', '?', '#'])
        .next()
        .map_or_else(String::new, str::to_lowercase)
}
