#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr;

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

// each case is one row of what LiteLLM's ProviderConfigManager reports for a SearchProviders value
#[rstest]
#[case::perplexity(0, "perplexity", "Perplexity", HttpMethod::Post, false)]
#[case::tavily(1, "tavily", "Tavily", HttpMethod::Post, false)]
#[case::parallel_ai(2, "parallel_ai", "Parallel AI", HttpMethod::Post, true)]
#[case::exa_ai(3, "exa_ai", "Exa AI", HttpMethod::Post, false)]
#[case::brave(4, "brave", "Brave Search", HttpMethod::Get, false)]
#[case::google_pse(5, "google_pse", "Google PSE", HttpMethod::Get, false)]
#[case::dataforseo(6, "dataforseo", "DataForSEO", HttpMethod::Post, false)]
#[case::firecrawl(7, "firecrawl", "Firecrawl", HttpMethod::Post, false)]
#[case::fastcrw(8, "fastcrw", "fastCRW", HttpMethod::Post, false)]
#[case::searxng(9, "searxng", "SearXNG", HttpMethod::Get, false)]
#[case::linkup(10, "linkup", "Linkup", HttpMethod::Post, false)]
#[case::duckduckgo(11, "duckduckgo", "DuckDuckGo", HttpMethod::Get, false)]
#[case::searchapi(
    12,
    "searchapi",
    "SearchAPI.io (Google Search)",
    HttpMethod::Get,
    false
)]
#[case::serper(13, "serper", "Serper", HttpMethod::Post, false)]
#[case::you_com(14, "you_com", "You.com", HttpMethod::Post, false)]
#[case::apiserpent(15, "apiserpent", "APISerpent", HttpMethod::Get, false)]
#[case::tinyfish(16, "tinyfish", "TinyFish", HttpMethod::Get, false)]
#[case::nimble(17, "nimble", "Nimble", HttpMethod::Post, false)]
fn every_registered_name_maps_to_its_config(
    #[case] position: usize,
    #[case] name: &str,
    #[case] display: &str,
    #[case] method: HttpMethod,
    #[case] rich: bool,
) {
    let environment = environment(&[]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let known = SearchProviderName::from_str(name).unwrap();
    assert_eq!(known, SearchProviderName::iter().nth(position).unwrap());
    assert_eq!(known.display_name(), display);
    let config = registry.search_config(name).unwrap();
    assert_eq!(config.name.display_name(), display);
    assert_eq!(config.http_method(), method);
    assert_eq!(config.supports_rich_search_input(), rich);
}

#[test]
fn the_registry_table_names_eighteen_providers() {
    assert_eq!(SearchProviderName::iter().count(), 18);
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
    assert!(SearchProviderName::from_str(name).is_err());
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

#[test]
fn registry_forwards_the_linkup_hooks() {
    let environment = environment(&[("LINKUP_API_KEY", "key")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("linkup").unwrap();
    assert_eq!(config.http_method(), HttpMethod::Post);
    let headers = config.validate_environment(Headers::new(), None, None);
    assert_eq!(
        headers.get("Authorization").map(String::as_str),
        Some("Bearer key")
    );
    let optional_params = params(r#"{"depth":"deep"}"#);
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let expected = serde_json::from_str::<JsonValue>(
        r#"{"q":"q","depth":"deep","outputType":"searchResults"}"#,
    )
    .unwrap();
    assert_eq!(body.0, expected);
    let url = config
        .complete_url(None, &optional_params, Some(&body), None)
        .unwrap();
    assert_eq!(url, "https://api.linkup.so/v1/search");
    let reply = config
        .transform_search_response(&response(
            200,
            &headers,
            r#"{"results":[{"name":"t","url":"https://example.test/1","content":"c"}]}"#,
        ))
        .unwrap();
    assert_eq!(reply.results.len(), 1);
    assert_eq!(reply.results[0].title, "t");
    let mut passed = Headers::new();
    passed.insert("X-Trace".to_owned(), "abc".to_owned());
    let (signed, encoded) = config
        .sign_request(
            passed,
            &OutboundSearch {
                api_base: "https://api.linkup.so/v1",
                api_key: None,
                optional_params: &optional_params,
                body: &body,
            },
        )
        .unwrap();
    assert_eq!(signed.get("X-Trace").map(String::as_str), Some("abc"));
    assert!(encoded.is_none());
}

#[test]
fn registry_forwards_the_searxng_hooks() {
    let environment = environment(&[("SEARXNG_API_BASE", "https://searx.test")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("searxng").unwrap();
    assert_eq!(config.http_method(), HttpMethod::Get);
    let headers = config.validate_environment(Headers::new(), None, None);
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    let optional_params = params(r#"{"country":"US"}"#);
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let url = config
        .complete_url(None, &optional_params, Some(&body), None)
        .unwrap();
    assert!(url.starts_with("https://searx.test/search?"));
    assert!(url.contains("q=q"));
    assert!(url.contains("format=json"));
    let reply = config
        .transform_search_response(&response(
            200,
            &headers,
            r#"{"results":[{"title":"t","url":"https://example.test/1","content":"c"}]}"#,
        ))
        .unwrap();
    assert_eq!(reply.results.len(), 1);
    assert_eq!(reply.results[0].url, "https://example.test/1");
}

#[test]
fn registry_forwards_the_google_pse_hooks() {
    let environment = environment(&[
        ("GOOGLE_PSE_API_KEY", "key"),
        ("GOOGLE_PSE_ENGINE_ID", "engine"),
    ]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("google_pse").unwrap();
    assert_eq!(config.http_method(), HttpMethod::Get);
    let mut passed = Headers::new();
    passed.insert("X-Trace".to_owned(), "abc".to_owned());
    let headers = config.validate_environment(passed, None, None);
    assert_eq!(headers.get("X-Trace").map(String::as_str), Some("abc"));
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    let optional_params = Object::new();
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let record = body.0.get("_google_pse_params").unwrap();
    assert_eq!(record.get("q").and_then(JsonValue::as_str), Some("q"));
    assert_eq!(record.get("cx").and_then(JsonValue::as_str), Some("engine"));
    let url = config
        .complete_url(None, &optional_params, Some(&body), None)
        .unwrap();
    assert!(url.starts_with("https://www.googleapis.com/customsearch/v1?"));
    assert!(url.contains("cx=engine"));
    let reply = config
        .transform_search_response(&response(
            200,
            &headers,
            r#"{"items":[{"title":"t","link":"https://example.test/1","snippet":"s"}]}"#,
        ))
        .unwrap();
    assert_eq!(reply.results.len(), 1);
    assert_eq!(reply.results[0].url, "https://example.test/1");
    let mut outgoing = Headers::new();
    outgoing.insert("X-Trace".to_owned(), "abc".to_owned());
    let (signed, encoded) = config
        .sign_request(
            outgoing,
            &OutboundSearch {
                api_base: "https://www.googleapis.com/customsearch/v1",
                api_key: None,
                optional_params: &optional_params,
                body: &body,
            },
        )
        .unwrap();
    assert_eq!(signed.get("X-Trace").map(String::as_str), Some("abc"));
    assert!(encoded.is_none());
}

#[test]
fn registry_forwards_the_searchapi_hooks() {
    let environment = environment(&[("SEARCHAPI_API_KEY", "key")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("searchapi").unwrap();
    assert_eq!(config.http_method(), HttpMethod::Get);
    let headers = config.validate_environment(Headers::new(), None, None);
    assert_eq!(
        headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    let optional_params = Object::new();
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let record = body.0.get("_searchapi_params").unwrap();
    assert_eq!(record.get("q").and_then(JsonValue::as_str), Some("q"));
    assert_eq!(
        record.get("engine").and_then(JsonValue::as_str),
        Some("google")
    );
    let url = config
        .complete_url(None, &optional_params, Some(&body), None)
        .unwrap();
    assert!(url.starts_with("https://www.searchapi.io/api/v1/search?"));
    assert!(url.contains("engine=google"));
    let reply = config
        .transform_search_response(&response(
            200,
            &headers,
            r#"{"organic_results":[{"title":"t","link":"https://example.test/1","snippet":"s"}]}"#,
        ))
        .unwrap();
    assert_eq!(reply.results.len(), 1);
    assert_eq!(reply.results[0].url, "https://example.test/1");
    let mut passed = Headers::new();
    passed.insert("X-Trace".to_owned(), "abc".to_owned());
    let (signed, encoded) = config
        .sign_request(
            passed,
            &OutboundSearch {
                api_base: "https://www.searchapi.io/api/v1/search",
                api_key: None,
                optional_params: &optional_params,
                body: &body,
            },
        )
        .unwrap();
    assert_eq!(signed.get("X-Trace").map(String::as_str), Some("abc"));
    assert!(encoded.is_none());
}
