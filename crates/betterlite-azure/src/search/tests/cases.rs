#![cfg(test)]

use rstest::rstest;

use super::{cap, citations, empty, max_results, nested, price, request, scope, settings};

#[rstest]
#[case::search_request_web_tool(request::search_request_web_tool)]
#[case::scope_token_header(scope::scope_token_header)]
#[case::citation_snippets(citations::citation_snippets)]
#[case::max_results_invalid_default(max_results::max_results_invalid_default)]
#[case::max_results_cap(cap::max_results_cap)]
#[case::empty_search_response(empty::empty_search_response)]
#[case::nested_upstream_error(nested::nested_upstream_error)]
#[case::search_settings(settings::search_settings)]
#[case::search_response_price(price::search_response_price)]
fn search_cases(#[case] case: fn()) {
    case();
}
