use betterlite_logging::python_json::dumps;
use betterlite_models::decisions::Json;
use betterlite_openai::search::provider::SearchQuery;

use crate::search::request::{AGENTCORE_MAX_QUERY_LENGTH, Params, transform_search_request};
use crate::search::tests::support::{MCP_BODY_TEXT, argument_dump, mcp_body, params, text_field};

pub(super) fn assert_query_list_and_200_char_cap() {
    assert_single_query_body();
    assert_list_query_is_joined();
    assert_query_is_capped_at_200_codepoints();
    assert_truncation_counts_codepoints();
    assert_max_results_pass_through();
    assert_no_extra_fields_are_copied();
}

fn single(text: &str) -> SearchQuery {
    SearchQuery::Single(text.to_owned())
}

fn request(query: &SearchQuery, optional: &Params) -> Json {
    transform_search_request(query, optional, None).unwrap()
}

fn query_text<'json>(body: &'json Json) -> &'json str {
    text_field(body, &["params", "arguments", "query"])
}

fn assert_single_query_body() {
    let body = request(&single("hello"), &Params::new());
    assert_eq!(dumps(&body), MCP_BODY_TEXT);
    assert_eq!(dumps(&mcp_body()), MCP_BODY_TEXT);
}

fn assert_list_query_is_joined() {
    let list = SearchQuery::Multiple(vec!["alpha".to_owned(), "beta".to_owned()]);
    assert_eq!(query_text(&request(&list, &Params::new())), "alpha beta");
    let empty = SearchQuery::Multiple(Vec::new());
    assert_eq!(query_text(&request(&empty, &Params::new())), "");
}

fn assert_query_is_capped_at_200_codepoints() {
    let long = "a".repeat(AGENTCORE_MAX_QUERY_LENGTH + 10);
    assert_eq!(
        query_text(&request(&single(&long), &Params::new())),
        "a".repeat(AGENTCORE_MAX_QUERY_LENGTH)
    );
    let exact = "b".repeat(AGENTCORE_MAX_QUERY_LENGTH);
    assert_eq!(query_text(&request(&single(&exact), &Params::new())), exact);
    let over = "c".repeat(AGENTCORE_MAX_QUERY_LENGTH + 1);
    assert_eq!(
        query_text(&request(&single(&over), &Params::new())),
        "c".repeat(AGENTCORE_MAX_QUERY_LENGTH)
    );
}

fn assert_truncation_counts_codepoints() {
    let accented = "\u{e9}".repeat(AGENTCORE_MAX_QUERY_LENGTH + 1);
    let body = request(&single(&accented), &Params::new());
    assert_eq!(
        query_text(&body),
        "\u{e9}".repeat(AGENTCORE_MAX_QUERY_LENGTH)
    );
    assert!(dumps(&body).contains("\\u00e9"));
    let astral = "\u{1d11e}".repeat(AGENTCORE_MAX_QUERY_LENGTH + 1);
    let body = request(&single(&astral), &Params::new());
    assert_eq!(
        query_text(&body),
        "\u{1d11e}".repeat(AGENTCORE_MAX_QUERY_LENGTH)
    );
    assert!(dumps(&body).contains("\\ud834\\udd1e"));
}

fn assert_max_results_pass_through() {
    let default = request(&single("hello"), &Params::new());
    assert_eq!(argument_dump(&default, "maxResults"), "10");
    let cases = [
        (r#"{"max_results":0}"#, "0"),
        (r#"{"max_results":-3}"#, "-3"),
        (r#"{"max_results":25}"#, "25"),
        (r#"{"max_results":30}"#, "30"),
        (r#"{"max_results":null}"#, "null"),
        (r#"{"max_results":"5"}"#, "\"5\""),
        (r#"{"max_results":1.5}"#, "1.5"),
    ];
    for (source, expected) in cases {
        let body = request(&single("hello"), &params(source));
        assert_eq!(argument_dump(&body, "maxResults"), expected);
    }
}

fn assert_no_extra_fields_are_copied() {
    let optional = params(
        r#"{"max_results":5,"tool_name":"packages___WebSearch","search_domain_filter":["x"],"country":"US"}"#,
    );
    let body = request(&single("hello"), &optional);
    assert_eq!(
        dumps(&body),
        r#"{"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "packages___WebSearch", "arguments": {"query": "hello", "maxResults": 5}}}"#
    );
}
