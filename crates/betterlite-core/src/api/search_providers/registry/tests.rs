#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, RequestInput, SearchProvider as _, SearchQuery,
};
use rstest::rstest;

use super::SearchRegistry;
use super::config::SearchBody;
use super::error::SearchConfigError;
use super::names::SearchProviderName;
use crate::api::search_providers::tinyfish::TINYFISH_RESULT_CAP;
use crate::search::SearchConfigLookup as _;

// what LiteLLM's ProviderConfigManager reports for each SearchProviders value
const EXPECTED: [(&str, &str, HttpMethod, bool); 18] = [
    ("perplexity", "Perplexity", HttpMethod::Post, false),
    ("tavily", "Tavily", HttpMethod::Post, false),
    ("parallel_ai", "Parallel AI", HttpMethod::Post, true),
    ("exa_ai", "Exa AI", HttpMethod::Post, false),
    ("brave", "Brave Search", HttpMethod::Get, false),
    ("google_pse", "Google PSE", HttpMethod::Get, false),
    ("dataforseo", "DataForSEO", HttpMethod::Post, false),
    ("firecrawl", "Firecrawl", HttpMethod::Post, false),
    ("fastcrw", "fastCRW", HttpMethod::Post, false),
    ("searxng", "SearXNG", HttpMethod::Get, false),
    ("linkup", "Linkup", HttpMethod::Post, false),
    ("duckduckgo", "DuckDuckGo", HttpMethod::Get, false),
    (
        "searchapi",
        "SearchAPI.io (Google Search)",
        HttpMethod::Get,
        false,
    ),
    ("serper", "Serper", HttpMethod::Post, false),
    ("you_com", "You.com", HttpMethod::Post, false),
    ("apiserpent", "APISerpent", HttpMethod::Get, false),
    ("tinyfish", "TinyFish", HttpMethod::Get, false),
    ("nimble", "Nimble", HttpMethod::Post, false),
];

fn environment(values: &[(&str, &str)]) -> BTreeMap<String, String> {
    values
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn params(json: &str) -> Object {
    serde_json::from_str(json).unwrap()
}

fn single(query: &str) -> SearchQuery {
    SearchQuery::Single(query.to_owned())
}

fn input<'call>(
    query: &'call SearchQuery,
    optional_params: &'call Object,
    headers: &'call Headers,
) -> RequestInput<'call> {
    RequestInput {
        query,
        optional_params,
        api_key: None,
        api_base: None,
        headers,
    }
}

fn response<'call>(
    status_code: u16,
    headers: &'call Headers,
    text: &'call str,
) -> ProviderResponse<'call> {
    ProviderResponse {
        status_code,
        headers,
        text,
    }
}

fn reply_with_results(count: usize) -> String {
    let items: Vec<String> = (0..count)
        .map(|index| {
            format!(r#"{{"title":"t{index}","url":"https://example.test/{index}","snippet":"s"}}"#)
        })
        .collect();
    format!(r#"{{"results":[{}]}}"#, items.join(","))
}

#[test]
fn every_registered_name_maps_to_its_config() {
    let environment = environment(&[]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    assert_eq!(EXPECTED.len(), SearchProviderName::iter().count());
    for (index, (name, display, method, rich)) in EXPECTED.into_iter().enumerate() {
        let known = SearchProviderName::from_name(name).unwrap();
        assert_eq!(known, SearchProviderName::iter().nth(index).unwrap());
        assert_eq!(known.display_name(), display);
        let config = registry.search_config(name).unwrap();
        assert_eq!(config.display_name(), display);
        assert_eq!(config.http_method(), method);
        assert_eq!(config.supports_rich_search_input(), rich);
    }
}

#[rstest]
#[case::agentcore("agentcore")]
#[case::bing_grounding("bing_grounding")]
#[case::display_name("TinyFish")]
#[case::empty("")]
#[case::padded(" tinyfish")]
#[case::separator("search_api")]
fn registry_returns_none_for_an_unregistered_name(#[case] name: &str) {
    let environment = environment(&[]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    assert!(registry.search_config(name).is_none());
    assert!(SearchProviderName::from_name(name).is_none());
}

#[test]
fn registry_returns_a_fresh_config_for_every_call() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let first = registry.search_config("tinyfish").unwrap();
    let second = registry.search_config("tinyfish").unwrap();
    let headers = Headers::new();
    let optional_params = params(r#"{"max_results":3}"#);
    first
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let text = reply_with_results(12);
    assert_eq!(
        first
            .transform_search_response(&response(200, &headers, &text))
            .unwrap()
            .results
            .len(),
        3
    );
    assert_eq!(
        second
            .transform_search_response(&response(200, &headers, &text))
            .unwrap()
            .results
            .len(),
        TINYFISH_RESULT_CAP
    );
}

#[test]
fn registry_forwards_the_tinyfish_hooks() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("tinyfish").unwrap();
    let mut headers = Headers::new();
    headers.insert("X-Trace".to_owned(), "abc".to_owned());
    let validated = config.validate_environment(headers.clone(), None, None);
    assert_eq!(validated.get("X-API-Key").map(String::as_str), Some("key"));
    assert_eq!(validated.get("X-Trace").map(String::as_str), Some("abc"));
    let optional_params = params(r#"{"country":"US"}"#);
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    assert_eq!(
        body.0
            .get("_tinyfish_params")
            .and_then(|record| record.get("query"))
            .and_then(JsonValue::as_str),
        Some("q")
    );
    let url = config
        .complete_url(None, &optional_params, Some(&body), None)
        .unwrap();
    assert_eq!(url, "https://api.search.tinyfish.ai?query=q&location=US");
    let reply = config
        .transform_search_response(&response(200, &headers, &reply_with_results(2)))
        .unwrap();
    assert_eq!(reply.results.len(), 2);
    let raw = reply.hidden_params.get("headers").unwrap();
    assert_eq!(raw.get("x-trace").and_then(JsonValue::as_str), Some("abc"));
    let error = config.http_error(&response(500, &headers, r#"{"error":{"message":"boom"}}"#));
    assert_eq!(
        error.message,
        "TinyFish Search: boom. See https://docs.tinyfish.ai/search-api for details."
    );
}

#[test]
fn registry_reports_a_missing_body_before_the_request_is_built() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("tinyfish").unwrap();
    let headers = Headers::new();
    let optional_params = Object::new();
    let empty = SearchBody(JsonValue::Null);
    let error = config
        .sign_request(
            headers,
            &OutboundSearch {
                api_base: "https://api.search.tinyfish.ai",
                api_key: None,
                optional_params: &optional_params,
                body: &empty,
            },
        )
        .unwrap_err();
    assert!(matches!(error, SearchConfigError::MissingBody));
    assert_eq!(
        error.to_string(),
        "the search request body was not built before the provider hook that needs it ran"
    );
}

#[test]
fn registry_signs_the_request_after_the_body_is_built() {
    let environment = environment(&[("TINYFISH_API_KEY", "key")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("tinyfish").unwrap();
    let headers = Headers::new();
    let optional_params = Object::new();
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let (signed, encoded) = config
        .sign_request(
            Headers::new(),
            &OutboundSearch {
                api_base: "https://api.search.tinyfish.ai",
                api_key: None,
                optional_params: &optional_params,
                body: &body,
            },
        )
        .unwrap();
    assert_eq!(signed.get("X-API-Key").map(String::as_str), Some("key"));
    assert!(encoded.is_none());
}

#[test]
fn registry_forwards_a_provider_error_class() {
    let environment = environment(&[]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("nimble").unwrap();
    let headers = Headers::new();
    let error = config.error_class("boom.", 400, &headers);
    assert!(error.message.starts_with("Nimble Search: boom. See "));
    assert!(error.message.ends_with(" for details."));
    assert_eq!(error.status_code, 400);
}

#[test]
fn registry_keeps_the_base_error_class_for_a_wrapper_provider() {
    let environment = environment(&[]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("google_pse").unwrap();
    let mut headers = Headers::new();
    headers.insert("x-request-id".to_owned(), "req-1".to_owned());
    let error = config.error_class("plain", 418, &headers);
    assert_eq!(error.message, "plain");
    assert_eq!(error.status_code, 418);
    assert_eq!(
        error.headers,
        Some(vec![("x-request-id".to_owned(), "req-1".to_owned())])
    );
    assert_eq!(
        config.http_error(&response(418, &headers, "plain")).message,
        "plain"
    );
}

#[test]
fn registry_keeps_headers_when_a_wrapper_cannot_validate() {
    let environment = environment(&[]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("google_pse").unwrap();
    let mut headers = Headers::new();
    headers.insert("X-Trace".to_owned(), "abc".to_owned());
    let validated = config.validate_environment(headers, None, None);
    assert_eq!(validated.get("X-Trace").map(String::as_str), Some("abc"));
}
