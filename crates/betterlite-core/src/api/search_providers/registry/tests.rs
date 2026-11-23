#![cfg(test)]

use std::collections::BTreeMap;
use std::str::FromStr as _;

use betterlite_config::sections::settings::JsonValue;
use betterlite_openai::openai_base_config::{Headers, Object};
use betterlite_openai::search::provider::{
    HttpMethod, OutboundSearch, ProviderResponse, RequestInput, SearchProvider as _, SearchQuery,
};
use rstest::rstest;
use strum::IntoEnumIterator as _;

use super::SearchRegistry;
use super::config::{SearchBody, SearchConfig};
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
fn signed_headers(
    config: &SearchConfig<'_>,
    api_base: &str,
    optional_params: &Object,
    body: &SearchBody,
) -> (Headers, Option<Vec<u8>>) {
    let mut passed = Headers::new();
    passed.insert("X-Trace".to_owned(), "abc".to_owned());
    config
        .sign_request(
            passed,
            &OutboundSearch {
                api_base,
                api_key: None,
                optional_params,
                body,
            },
        )
        .unwrap()
}

// each case is one row of what LiteLLM's ProviderConfigManager and the provider's own config report
struct ProviderCase {
    position: usize,
    name: &'static str,
    display: &'static str,
    method: HttpMethod,
    rich: bool,
    endpoint: &'static str,
}

#[rstest]
#[case::perplexity(ProviderCase {
    position: 0,
    name: "perplexity",
    display: "Perplexity",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://api.perplexity.ai/search",
})]
#[case::tavily(ProviderCase {
    position: 1,
    name: "tavily",
    display: "Tavily",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://api.tavily.com/search",
})]
#[case::parallel_ai(ProviderCase {
    position: 2,
    name: "parallel_ai",
    display: "Parallel AI",
    method: HttpMethod::Post,
    rich: true,
    endpoint: "https://api.parallel.ai/v1/search",
})]
#[case::exa_ai(ProviderCase {
    position: 3,
    name: "exa_ai",
    display: "Exa AI",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://api.exa.ai/search",
})]
#[case::brave(ProviderCase {
    position: 4,
    name: "brave",
    display: "Brave Search",
    method: HttpMethod::Get,
    rich: false,
    endpoint: "https://api.search.brave.com/res/v1/web/search",
})]
#[case::google_pse(ProviderCase {
    position: 5,
    name: "google_pse",
    display: "Google PSE",
    method: HttpMethod::Get,
    rich: false,
    endpoint: "https://www.googleapis.com/customsearch/v1",
})]
#[case::dataforseo(ProviderCase {
    position: 6,
    name: "dataforseo",
    display: "DataForSEO",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://api.dataforseo.com/v3/serp/google/organic/live/advanced",
})]
#[case::firecrawl(ProviderCase {
    position: 7,
    name: "firecrawl",
    display: "Firecrawl",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://api.firecrawl.dev/v2/search",
})]
#[case::fastcrw(ProviderCase {
    position: 8,
    name: "fastcrw",
    display: "fastCRW",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://fastcrw.com/api/v1/search",
})]
#[case::searxng(ProviderCase {
    position: 9,
    name: "searxng",
    display: "SearXNG",
    method: HttpMethod::Get,
    rich: false,
    endpoint: "https://searx.test/search",
})]
#[case::linkup(ProviderCase {
    position: 10,
    name: "linkup",
    display: "Linkup",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://api.linkup.so/v1/search",
})]
#[case::duckduckgo(ProviderCase {
    position: 11,
    name: "duckduckgo",
    display: "DuckDuckGo",
    method: HttpMethod::Get,
    rich: false,
    endpoint: "https://api.duckduckgo.com",
})]
#[case::searchapi(ProviderCase {
    position: 12,
    name: "searchapi",
    display: "SearchAPI.io (Google Search)",
    method: HttpMethod::Get,
    rich: false,
    endpoint: "https://www.searchapi.io/api/v1/search",
})]
#[case::serper(ProviderCase {
    position: 13,
    name: "serper",
    display: "Serper",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://google.serper.dev/search",
})]
#[case::you_com(ProviderCase {
    position: 14,
    name: "you_com",
    display: "You.com",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://api.you.com/v1/agents/search",
})]
#[case::apiserpent(ProviderCase {
    position: 15,
    name: "apiserpent",
    display: "APISerpent",
    method: HttpMethod::Get,
    rich: false,
    endpoint: "https://apiserpent.com/api/search/quick",
})]
#[case::tinyfish(ProviderCase {
    position: 16,
    name: "tinyfish",
    display: "TinyFish",
    method: HttpMethod::Get,
    rich: false,
    endpoint: "https://api.search.tinyfish.ai",
})]
#[case::nimble(ProviderCase {
    position: 17,
    name: "nimble",
    display: "Nimble",
    method: HttpMethod::Post,
    rich: false,
    endpoint: "https://sdk.nimbleway.com/v2/search",
})]
fn every_registered_name_maps_to_its_config(#[case] provider: ProviderCase) {
    let environment = environment(&[
        ("LINKUP_API_KEY", "key"),
        ("SEARXNG_API_BASE", "https://searx.test"),
    ]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let known = SearchProviderName::from_str(provider.name).unwrap();
    assert_eq!(
        known,
        SearchProviderName::iter().nth(provider.position).unwrap()
    );
    assert_eq!(known.display_name(), provider.display);
    let config = registry.search_config(provider.name).unwrap();
    assert_eq!(config.name.display_name(), provider.display);
    assert_eq!(config.http_method(), provider.method);
    assert_eq!(config.supports_rich_search_input(), provider.rich);
    // the endpoint only this provider's own config builds, so a swapped arm changes it
    assert_eq!(
        config
            .complete_url(None, &Object::new(), None, None)
            .unwrap(),
        provider.endpoint
    );
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
    // the transport lowercases response header names before the reply hook sees them
    headers.insert("x-trace".to_owned(), "abc".to_owned());
    let validated = config.validate_environment(headers.clone(), None, None);
    assert_eq!(validated.get("X-API-Key").map(String::as_str), Some("key"));
    assert_eq!(validated.get("x-trace").map(String::as_str), Some("abc"));
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
}
#[test]
fn registry_forwards_the_linkup_reply_hooks() {
    let environment = environment(&[("LINKUP_API_KEY", "key")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("linkup").unwrap();
    let headers = config.validate_environment(Headers::new(), None, None);
    let optional_params = params(r#"{"depth":"deep"}"#);
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let reply = config
        .transform_search_response(&response(
            200,
            &headers,
            r#"{"results":[{"name":"t","url":"https://example.test/1","content":"c"}]}"#,
        ))
        .unwrap();
    assert_eq!(reply.results.len(), 1);
    assert_eq!(reply.results[0].title, "t");
    let (signed, encoded) =
        signed_headers(&config, "https://api.linkup.so/v1", &optional_params, &body);
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
}
#[test]
fn registry_forwards_the_google_pse_reply_hooks() {
    let environment = environment(&[
        ("GOOGLE_PSE_API_KEY", "key"),
        ("GOOGLE_PSE_ENGINE_ID", "engine"),
    ]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("google_pse").unwrap();
    let headers = config.validate_environment(Headers::new(), None, None);
    let optional_params = Object::new();
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let reply = config
        .transform_search_response(&response(
            200,
            &headers,
            r#"{"items":[{"title":"t","link":"https://example.test/1","snippet":"s"}]}"#,
        ))
        .unwrap();
    assert_eq!(reply.results.len(), 1);
    assert_eq!(reply.results[0].url, "https://example.test/1");
    let (signed, encoded) = signed_headers(
        &config,
        "https://www.googleapis.com/customsearch/v1",
        &optional_params,
        &body,
    );
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
}
#[test]
fn registry_forwards_the_searchapi_reply_hooks() {
    let environment = environment(&[("SEARCHAPI_API_KEY", "key")]);
    let registry = SearchRegistry {
        environment: &environment,
    };
    let config = registry.search_config("searchapi").unwrap();
    let headers = config.validate_environment(Headers::new(), None, None);
    let optional_params = Object::new();
    let body = config
        .transform_search_request(&input(&single("q"), &optional_params, &headers))
        .unwrap();
    let reply = config
        .transform_search_response(&response(
            200,
            &headers,
            r#"{"organic_results":[{"title":"t","link":"https://example.test/1","snippet":"s"}]}"#,
        ))
        .unwrap();
    assert_eq!(reply.results.len(), 1);
    assert_eq!(reply.results[0].url, "https://example.test/1");
    let (signed, encoded) = signed_headers(
        &config,
        "https://www.searchapi.io/api/v1/search",
        &optional_params,
        &body,
    );
    assert_eq!(signed.get("X-Trace").map(String::as_str), Some("abc"));
    assert!(encoded.is_none());
}
