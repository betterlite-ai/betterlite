#![cfg(test)]
use std::collections::BTreeMap;
use std::sync::Mutex;

use secrecy::{ExposeSecret as _, SecretString};
use serde_json::Error;

use crate::openai_base_config::{Headers, Object};
use crate::search::provider::{
    HttpMethod, OutboundSearch, PERPLEXITY_PARAMS, ProviderResponse, SearchProvider, SearchQuery,
};
use crate::search::records::SearchResponse;
#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenRequest {
    api_key: Option<String>,
    api_base: Option<String>,
    headers: Headers,
}

struct EchoSearch {
    seen: Mutex<Option<SeenRequest>>,
}

fn echo() -> EchoSearch {
    EchoSearch {
        seen: Mutex::new(None),
    }
}

impl SearchProvider for EchoSearch {
    type Body = SearchQuery;
    type Error = Error;

    fn complete_url(
        &self,
        api_base: Option<&str>,
        _optional_params: &Object,
        _body: Option<&SearchQuery>,
        _api_key: Option<&SecretString>,
    ) -> Result<String, Error> {
        let base = api_base.unwrap_or("https://echo.example");
        Ok(format!("{base}/search"))
    }

    fn transform_search_request(
        &self,
        query: &SearchQuery,
        _optional_params: &Object,
        api_key: Option<&SecretString>,
        api_base: Option<&str>,
        headers: &Headers,
    ) -> Result<SearchQuery, Error> {
        *self.seen.lock().unwrap() = Some(SeenRequest {
            api_key: api_key.map(|key| key.expose_secret().to_owned()),
            api_base: api_base.map(str::to_owned),
            headers: headers.clone(),
        });
        Ok(query.clone())
    }

    fn transform_search_response(
        &self,
        response: &ProviderResponse<'_>,
    ) -> Result<SearchResponse, Error> {
        serde_json::from_str(response.text)
    }
}
fn headers(name: &str, value: &str) -> Headers {
    BTreeMap::from([(name.to_owned(), value.to_owned())])
}
#[test]
fn base_defaults_match_litellm() {
    assert_eq!(EchoSearch::ui_friendly_name(), "Unknown Search Provider");
    assert!(!echo().supports_rich_search_input());
    assert_eq!(echo().http_method(), HttpMethod::Post);
    let expected = "max_results search_domain_filter country max_tokens_per_page";
    assert_eq!(PERPLEXITY_PARAMS.join(" "), expected);
}
#[test]
fn base_hooks_pass_headers_through_unsigned() {
    let sent = headers("content-type", "application/json");
    let key = SecretString::from("sk-search");
    let kept = echo().validate_environment(sent.clone(), Some(&key), None);
    assert_eq!(kept, sent);
    let query = SearchQuery::Single(String::from("rust"));
    let request = OutboundSearch {
        api_base: "https://echo.example",
        api_key: Some(&key),
        optional_params: &Object::new(),
        body: &query,
    };
    assert_eq!(echo().sign_request(kept, &request).unwrap(), (sent, None));
}
#[test]
fn provider_hooks_build_url_body_and_response() {
    let single: SearchQuery = serde_json::from_str(r#""latest rust""#).unwrap();
    assert_eq!(single, SearchQuery::Single(String::from("latest rust")));
    let many: SearchQuery = serde_json::from_str(r#"["a", "b"]"#).unwrap();
    let body = echo()
        .transform_search_request(&many, &Object::new(), None, None, &Headers::new())
        .unwrap();
    assert_eq!(serde_json::to_string(&body).unwrap(), r#"["a","b"]"#);
    let url = echo().complete_url(None, &Object::new(), Some(&body), None);
    assert_eq!(url.unwrap(), "https://echo.example/search");
    let response = ProviderResponse {
        status_code: 200,
        headers: &Headers::new(),
        text: r#"{"results": []}"#,
    };
    let parsed = echo().transform_search_response(&response).unwrap();
    assert!(parsed.results.is_empty());
}
#[test]
fn search_errors_carry_status_text_and_headers() {
    let retry = headers("retry-after", "3");
    let response = ProviderResponse {
        status_code: 429,
        headers: &retry,
        text: "slow down",
    };
    let error = echo().http_error(&response);
    assert_eq!(
        (error.status_code, error.message.as_str()),
        (429, "slow down")
    );
    assert_eq!(error.headers, Some(retry.into_iter().collect()));
    assert!(!error.status_code_is_synthesized);
    let direct = echo().error_class("bad query", 400, &Headers::new());
    assert_eq!(
        (direct.status_code, direct.message.as_str(), direct.headers),
        (400, "bad query", Some(Vec::new()))
    );
}

#[test]
fn transform_sees_key_base_and_validated_headers() {
    let key = SecretString::from("sk-search");
    let validated = headers("x-validated", "yes");
    let provider = echo();
    let body = provider
        .transform_search_request(
            &SearchQuery::Single(String::from("rust")),
            &Object::new(),
            Some(&key),
            Some("https://record.example"),
            &validated,
        )
        .unwrap();
    assert_eq!(body, SearchQuery::Single(String::from("rust")));
    let seen = provider.seen.lock().unwrap().clone().unwrap();
    assert_eq!(seen.api_key.as_deref(), Some("sk-search"));
    assert_eq!(seen.api_base.as_deref(), Some("https://record.example"));
    assert_eq!(seen.headers, validated);
}
