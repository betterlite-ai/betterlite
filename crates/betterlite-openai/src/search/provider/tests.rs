#![cfg(test)]

use std::collections::BTreeMap;

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Error;

use crate::openai_base_config::{Headers, Object};
use crate::search::provider::{
    HttpMethod, OutboundSearch, PERPLEXITY_PARAMS, ProviderResponse, SearchProvider, SearchQuery,
};
use crate::search::records::SearchResponse;

struct EchoSearch;

impl SearchProvider for EchoSearch {
    type Body = SearchQuery;
    type Error = Error;

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchQuery>,
    ) -> Result<String, Error> {
        Ok(format!(
            "{}/search",
            api_base.unwrap_or("https://echo.example")
        ))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        _optional_params: &Object,
    ) -> Result<SearchQuery, Error> {
        Ok(query.clone())
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Error> {
        serde_json::from_str(response.text)
    }
}

struct KeyedSearch;

impl SearchProvider for KeyedSearch {
    type Body = SearchQuery;
    type Error = Error;

    fn ui_friendly_name() -> &'static str {
        "Keyed"
    }

    fn validate_environment(
        &self,
        mut headers: Headers,
        api_key: Option<&SecretString>,
        _api_base: Option<&str>,
    ) -> Headers {
        if let Some(key) = api_key {
            headers.insert(String::from("x-api-key"), key.expose_secret().to_owned());
        }
        headers
    }

    fn complete_url(
        &self,
        _api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchQuery>,
    ) -> Result<String, Error> {
        Ok(String::from("https://keyed.example/search"))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        _optional_params: &Object,
    ) -> Result<SearchQuery, Error> {
        Ok(query.clone())
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Error> {
        serde_json::from_str(response.text)
    }
}

fn headers() -> Headers {
    BTreeMap::from([(
        String::from("content-type"),
        String::from("application/json"),
    )])
}

#[test]
fn base_defaults_match_litellm() {
    assert_eq!(EchoSearch::ui_friendly_name(), "Unknown Search Provider");
    assert_eq!(KeyedSearch::ui_friendly_name(), "Keyed");
    assert!(!EchoSearch.supports_rich_search_input());
    assert_eq!(EchoSearch.http_method(), HttpMethod::Post);
    assert_eq!(serde_json::to_string(&HttpMethod::Get).unwrap(), r#""GET""#);
    assert_eq!(
        PERPLEXITY_PARAMS,
        [
            "max_results",
            "search_domain_filter",
            "country",
            "max_tokens_per_page"
        ]
    );
}

#[test]
fn validate_environment_keeps_headers_unless_overridden() {
    let key = SecretString::from("sk-search");
    assert_eq!(
        EchoSearch.validate_environment(headers(), Some(&key), None),
        headers()
    );
    let keyed = KeyedSearch.validate_environment(headers(), Some(&key), None);
    assert_eq!(
        keyed.get("x-api-key").map(String::as_str),
        Some("sk-search")
    );
    assert_eq!(keyed.len(), 2);
}

#[test]
fn sign_request_leaves_body_unsigned() {
    let query = SearchQuery::Multiple(vec![String::from("rust"), String::from("serde")]);
    let request = OutboundSearch {
        api_base: "https://echo.example",
        api_key: None,
        optional_params: &Object::new(),
        body: &query,
    };
    assert_eq!(
        EchoSearch.sign_request(headers(), &request).unwrap(),
        (headers(), None)
    );
}

#[test]
fn provider_hooks_build_url_body_and_response() {
    let query: SearchQuery = serde_json::from_str(r#""latest rust release""#).unwrap();
    assert_eq!(
        query,
        SearchQuery::Single(String::from("latest rust release"))
    );
    let many: SearchQuery = serde_json::from_str(r#"["a", "b"]"#).unwrap();
    let body = EchoSearch
        .transform_search_request(&many, &Object::new())
        .unwrap();
    assert_eq!(serde_json::to_string(&body).unwrap(), r#"["a","b"]"#);
    assert_eq!(
        EchoSearch
            .complete_url(None, &Object::new(), Some(&body))
            .unwrap(),
        "https://echo.example/search"
    );
    let response = ProviderResponse {
        status_code: 200,
        headers: &headers(),
        text: r#"{"results": []}"#,
    };
    let parsed = KeyedSearch.transform_search_response(&response).unwrap();
    assert!(parsed.results.is_empty());
}

#[test]
fn search_errors_carry_status_text_and_headers() {
    let response_headers = BTreeMap::from([(String::from("retry-after"), String::from("3"))]);
    let response = ProviderResponse {
        status_code: 429,
        headers: &response_headers,
        text: "slow down",
    };
    let error = EchoSearch.http_error(&response);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert_eq!(
        error.headers,
        Some(vec![(String::from("retry-after"), String::from("3"))])
    );
    assert!(!error.status_code_is_synthesized);
    assert_eq!(error.response.status_code, 429);
    let direct = EchoSearch.error_class("bad query", 400, &Headers::new());
    assert_eq!(
        (direct.status_code, direct.message.as_str(), direct.headers),
        (400, "bad query", Some(Vec::new()))
    );
}
