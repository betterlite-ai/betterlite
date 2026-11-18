use rstest::rstest;

use crate::agentcore_search_reply::body::parse_mcp_body;
use crate::agentcore_search_reply::error::SearchReplyError;
use crate::agentcore_search_reply::tests::support::{
    decode, decode_bytes, decode_with, headers, json, provider_headers, provider_message,
    provider_status, titles,
};
use crate::bedrock_settings::ErrorHeaders;

#[rstest]
#[case::mcp_json_results(mcp_json_results)]
#[case::mcp_structured_fallback(mcp_structured_fallback)]
#[case::search_result_fields(search_result_fields)]
#[case::mcp_sse_results(mcp_sse_results)]
#[case::mcp_sse_no_frame(mcp_sse_no_frame)]
#[case::mcp_error_precedence(mcp_error_precedence)]
#[case::mcp_tool_error(mcp_tool_error)]
#[case::agentcore_search_reply_error(agentcore_search_reply_error)]
fn agentcore_search_reply_cases(#[case] case: fn()) {
    case();
}

fn mcp_json_results() {
    let sse_content_type = [(
        String::from("content-type"),
        String::from("text/event-stream"),
    )];
    let list = r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"a\",\"url\":\"u1\"},{\"title\":\"b\"}]"}]}}"#;
    let response = decode_with(list, 200, &sse_content_type).unwrap();
    assert_eq!(titles(&response), vec!["a", "b"]);

    let results_field =
        r#"{"result":{"content":[{"type":"text","text":"{\"results\":[{\"title\":\"c\"}]}"}]}}"#;
    assert_eq!(titles(&decode(results_field, 200).unwrap()), vec!["c"]);

    let other_field =
        r#"{"result":{"content":[{"type":"text","text":"{\"other\":[{\"title\":\"x\"}]}"}]}}"#;
    assert!(decode(other_field, 200).unwrap().results.is_empty());

    let mixed = r#"{"result":{"content":[{"type":"text","text":"[1,\"x\",{\"title\":\"ok\"}]"}]}}"#;
    assert_eq!(titles(&decode(mixed, 200).unwrap()), vec!["ok"]);

    let nonstandard =
        r#"{"result":{"content":[{"type":"text","text":"[NaN,{\"title\":\"x\"}]"}]}}"#;
    assert!(decode(nonstandard, 200).unwrap().results.is_empty());

    let blocks = r#"{"result":{"content":[7,"x",{"type":"image","text":"[{\"title\":\"no\"}]"},{"type":"text","text":"no json"},{"type":"text","text":9}]}}"#;
    assert!(decode(blocks, 200).unwrap().results.is_empty());

    let result_list = r#"{"result":[{"title":"no"}]}"#;
    assert!(decode(result_list, 200).unwrap().results.is_empty());

    let content_text = r#"{"result":{"content":"[{\"title\":\"no\"}]"}}"#;
    assert!(decode(content_text, 200).unwrap().results.is_empty());

    let leading = " \n\t{\"result\":{}}";
    assert!(decode(leading, 200).unwrap().results.is_empty());

    assert!(matches!(
        decode(r#"{"result":"#, 200),
        Err(SearchReplyError::Json { .. })
    ));
    assert!(matches!(
        decode("[1,2]", 200),
        Err(SearchReplyError::NotObject)
    ));
}

fn mcp_structured_fallback() {
    let prose = r#"{"result":{"content":[{"type":"text","text":"prose only"}],"structuredContent":[{"title":"s1"},{"title":"s2"}]}}"#;
    assert_eq!(titles(&decode(prose, 200).unwrap()), vec!["s1", "s2"]);

    let structured_field = r#"{"result":{"content":[{"type":"text","text":"prose"}],"structuredContent":{"results":[{"title":"r"}]}}}"#;
    assert_eq!(titles(&decode(structured_field, 200).unwrap()), vec!["r"]);

    let empty_map = r#"{"result":{"content":[{"type":"text","text":"[{}]"}],"structuredContent":[{"title":"s"}]}}"#;
    let response = decode(empty_map, 200).unwrap();
    assert_eq!(response.results.len(), 1);
    assert_eq!(response.results[0].title, "");

    let empty_list = r#"{"result":{"content":[{"type":"text","text":"[]"}],"structuredContent":[{"title":"s"}]}}"#;
    assert_eq!(titles(&decode(empty_list, 200).unwrap()), vec!["s"]);

    let ordered = r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"first\"}]"},{"type":"text","text":"[{\"title\":\"second\"},{\"title\":\"first\"}]"}]}}"#;
    assert_eq!(
        titles(&decode(ordered, 200).unwrap()),
        vec!["first", "second", "first"]
    );

    let not_a_list =
        r#"{"result":{"content":[{"type":"text","text":"x"}],"structuredContent":"no"}}"#;
    assert!(decode(not_a_list, 200).unwrap().results.is_empty());

    let absent = r#"{"result":{"content":[{"type":"text","text":"x"}]}}"#;
    assert!(decode(absent, 200).unwrap().results.is_empty());

    let structured_object = r#"{"result":{"structuredContent":{"other":[{"title":"x"}]}}}"#;
    assert!(decode(structured_object, 200).unwrap().results.is_empty());

    let structured_scalar = r#"{"result":{"structuredContent":{"results":7}}}"#;
    assert!(decode(structured_scalar, 200).unwrap().results.is_empty());
}

fn search_result_fields() {
    let body = r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"T\",\"url\":\"U\",\"text\":\"snippet text\",\"publishedDate\":\"2024-01-02\",\"last_updated\":\"ignored\"},{\"title\":\"\",\"snippet\":\"from snippet\",\"date\":\"2024-02-03\"},{\"title\":\"  \",\"text\":\"  \",\"publishedDate\":\"\",\"date\":\"fallback\"},{\"title\":7,\"url\":null,\"snippet\":5,\"date\":false},{\"text\":\"\",\"snippet\":\"fallback snippet\"}]"}]}}"#;
    let response = decode(body, 200).unwrap();
    assert_eq!(response.object.0, "search");
    assert_eq!(response.results.len(), 5);

    assert_eq!(response.results[0].title, "T");
    assert_eq!(response.results[0].url, "U");
    assert_eq!(response.results[0].snippet, "snippet text");
    assert_eq!(response.results[0].date.as_deref(), Some("2024-01-02"));
    assert_eq!(response.results[0].last_updated, None);
    assert!(response.results[0].extra.is_empty());

    assert_eq!(response.results[1].title, "");
    assert_eq!(response.results[1].url, "");
    assert_eq!(response.results[1].snippet, "from snippet");
    assert_eq!(response.results[1].date.as_deref(), Some("2024-02-03"));

    assert_eq!(response.results[2].title, "  ");
    assert_eq!(response.results[2].snippet, "  ");
    assert_eq!(response.results[2].date.as_deref(), Some("fallback"));

    assert_eq!(response.results[3].title, "");
    assert_eq!(response.results[3].url, "");
    assert_eq!(response.results[3].snippet, "");
    assert_eq!(response.results[3].date, None);
    assert_eq!(response.results[3].last_updated, None);

    assert_eq!(response.results[4].title, "");
    assert_eq!(response.results[4].snippet, "fallback snippet");

    let precedence = r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"p\",\"text\":\"from text\",\"snippet\":\"from snippet\",\"publishedDate\":\"2024-01-01\",\"date\":\"2024-02-02\"}]"}]}}"#;
    let precedence_response = decode(precedence, 200).unwrap();
    assert_eq!(precedence_response.results.len(), 1);
    assert_eq!(precedence_response.results[0].snippet, "from text");
    assert_eq!(
        precedence_response.results[0].date.as_deref(),
        Some("2024-01-01")
    );
}

fn mcp_sse_results() {
    let single = r#"event: message
data: {"result":{"content":[{"type":"text","text":"[{\"title\":\"sse\"}]"}]}}

"#;
    assert_eq!(titles(&decode(single, 200).unwrap()), vec!["sse"]);

    let multiline = r#"data: {"result":
data: {"content":[{"type":"text","text":"[{\"title\":\"joined\"}]"}]}}

"#;
    assert_eq!(titles(&decode(multiline, 200).unwrap()), vec!["joined"]);

    let first = r#"data: {"jsonrpc":"2.0","id":1,"result":{"content":[{"type":"text","text":"[{\"title\":\"first\"}]"}]}}

data: {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"[{\"title\":\"second\"}]"}]}}

"#;
    assert_eq!(titles(&decode(first, 200).unwrap()), vec!["first"]);

    let junk = r#"event: ping

data: not json

data: [1,2]

data:    {"result":{"content":[{"type":"text","text":"[{\"title\":\"kept\"}]"}]}}

"#;
    assert_eq!(titles(&decode(junk, 200).unwrap()), vec!["kept"]);

    let notifications = r#"data: {"jsonrpc":"2.0","method":"notifications/progress"}

data: {"jsonrpc":"2.0","method":"notifications/message"}

"#;
    assert_eq!(
        parse_mcp_body(notifications, &headers()).unwrap(),
        json(r#"{"jsonrpc":"2.0","method":"notifications/message"}"#)
    );

    let indented = r#"  data: {"result":{}}

"#;
    assert_eq!(provider_status(&decode(indented, 200).unwrap_err()), 502);

    let comment =
        ": keepalive\n\ndata: {\"result\":{\"structuredContent\":[{\"title\":\"c\"}]}}\n\n";
    assert_eq!(titles(&decode(comment, 200).unwrap()), vec!["c"]);

    let identified = "id: 7\ndata: {\"result\":{\"structuredContent\":[{\"title\":\"i\"}]}}\n\n";
    assert_eq!(titles(&decode(identified, 200).unwrap()), vec!["i"]);

    let retried = "retry: 1000\ndata: {\"result\":{\"structuredContent\":[{\"title\":\"r\"}]}}\n\n";
    assert_eq!(titles(&decode(retried, 200).unwrap()), vec!["r"]);

    let padded = " data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\ndata: {\"result\":{\"structuredContent\":[{\"title\":\"padded\"}]}}\n\n";
    assert_eq!(titles(&decode(padded, 200).unwrap()), vec!["padded"]);

    let separated = "\u{1c}data: {\"result\":{\"structuredContent\":[{\"title\":\"sep\"}]}}";
    assert_eq!(titles(&decode(separated, 200).unwrap()), vec!["sep"]);

    let unit = "\u{1f}data: {\"result\":{}}";
    assert_eq!(provider_status(&decode(unit, 200).unwrap_err()), 502);

    let spaced = "data: {\"result\":{\"structuredContent\":[{\"title\":\"first\"}]}}\n \t\ndata: {\"result\":{\"structuredContent\":[{\"title\":\"second\"}]}}\n\n";
    assert_eq!(titles(&decode(spaced, 200).unwrap()), vec!["first"]);

    let crlf = "data: {\"result\":{\"structuredContent\":[{\"title\":\"crlf\"}]}}\r\n\r\n";
    assert_eq!(titles(&decode(crlf, 200).unwrap()), vec!["crlf"]);

    let error_first = r#"data: {"error":{}}

data: {"result":{"structuredContent":[{"title":"later"}]}}

"#;
    let error_first_failure = decode(error_first, 200).unwrap_err();
    assert_eq!(provider_status(&error_first_failure), 502);
    assert_eq!(
        provider_message(&error_first_failure),
        "AgentCore gateway MCP error: {}"
    );

    let null_error_first = r#"data: {"error":null}

data: {"result":{"structuredContent":[{"title":"later"}]}}

"#;
    assert!(decode(null_error_first, 200).unwrap().results.is_empty());

    for boundary in [
        '\u{0b}', '\u{0c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}',
    ] {
        let body = format!("data: {{\"result\":{boundary}data: {{}}}}");
        assert!(
            decode(&body, 200).unwrap().results.is_empty(),
            "{boundary:?}"
        );
    }
}

fn mcp_sse_no_frame() {
    let no_frame = "data: not json\n\ndata: [1]\n\n";
    let error = decode(no_frame, 200).unwrap_err();
    assert_eq!(
        provider_message(&error),
        format!("AgentCore gateway returned SSE without a JSON data frame: {no_frame}")
    );
    assert_eq!(provider_status(&error), 502);
    assert_eq!(
        provider_headers(&error),
        Some(&ErrorHeaders::Http(headers()))
    );

    let long = format!("data: {}", "x".repeat(300));
    let long_error = decode(&long, 500).unwrap_err();
    assert_eq!(provider_status(&long_error), 502);
    let expected: String = long.chars().take(200).collect();
    assert_eq!(
        provider_message(&long_error),
        format!("AgentCore gateway returned SSE without a JSON data frame: {expected}")
    );

    let exactly = format!("data: {}", "y".repeat(194));
    let exactly_error = decode(&exactly, 200).unwrap_err();
    assert_eq!(
        provider_message(&exactly_error),
        format!("AgentCore gateway returned SSE without a JSON data frame: {exactly}")
    );

    let over = format!("data: {}", "y".repeat(195));
    let over_error = decode(&over, 200).unwrap_err();
    let cut: String = over.chars().take(200).collect();
    assert_eq!(
        provider_message(&over_error),
        format!("AgentCore gateway returned SSE without a JSON data frame: {cut}")
    );

    assert!(matches!(
        decode("not json", 200),
        Err(SearchReplyError::Json { .. })
    ));
    assert!(matches!(
        decode("", 200),
        Err(SearchReplyError::Json { .. })
    ));

    let unterminated = r#"data: {"result":{}}"#;
    assert!(decode(unterminated, 200).unwrap().results.is_empty());
}

fn mcp_error_precedence() {
    let with_result =
        r#"{"error":{},"result":{"content":[{"type":"text","text":"[{\"title\":\"no\"}]"}]}}"#;
    let error = decode(with_result, 200).unwrap_err();
    assert_eq!(provider_status(&error), 502);
    assert_eq!(provider_message(&error), "AgentCore gateway MCP error: {}");
    assert_eq!(
        provider_headers(&error),
        Some(&ErrorHeaders::Http(headers()))
    );

    let error_false = decode(r#"{"error":false}"#, 200).unwrap_err();
    assert_eq!(provider_status(&error_false), 502);

    let rendered = decode(r#"{"error":{"code":-32000,"message":"boom"}}"#, 200).unwrap_err();
    assert_eq!(
        provider_message(&rendered),
        "AgentCore gateway MCP error: {\"code\":-32000,\"message\":\"boom\"}"
    );

    let raw_status = decode(r#"{"error":{}}"#, 503).unwrap_err();
    assert_eq!(provider_status(&raw_status), 503);

    let null_error =
        r#"{"error":null,"result":{"content":[{"type":"text","text":"[{\"title\":\"kept\"}]"}]}}"#;
    assert_eq!(titles(&decode(null_error, 200).unwrap()), vec!["kept"]);

    let tool_error =
        r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"no\"}]"}],"isError":true}}"#;
    let in_band_error = decode(tool_error, 200).unwrap_err();
    assert_eq!(provider_status(&in_band_error), 502);
    assert_eq!(in_band_error.status(), Some(502));
    assert_eq!(
        provider_headers(&in_band_error),
        Some(&ErrorHeaders::Http(headers()))
    );
    assert_eq!(
        provider_message(&in_band_error),
        "AgentCore web search tool error: [{\"title\":\"no\"}]"
    );

    let tool_raw = r#"{"result":{"content":[{"type":"text","text":"x"}],"isError":1}}"#;
    assert_eq!(provider_status(&decode(tool_raw, 500).unwrap_err()), 500);

    let falsy =
        r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"kept\"}]"}],"isError":0}}"#;
    assert_eq!(titles(&decode(falsy, 200).unwrap()), vec!["kept"]);

    let at_500 = r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"ok\"}]"}]}}"#;
    assert_eq!(titles(&decode(at_500, 500).unwrap()), vec!["ok"]);

    let error_zero = decode(r#"{"error":0}"#, 200).unwrap_err();
    assert_eq!(provider_status(&error_zero), 502);

    let error_empty = decode(r#"{"error":[]}"#, 500).unwrap_err();
    assert_eq!(provider_status(&error_empty), 500);

    let at_400 = decode(r#"{"error":{}}"#, 400).unwrap_err();
    assert_eq!(provider_status(&at_400), 400);

    let at_399 = decode(r#"{"error":{}}"#, 399).unwrap_err();
    assert_eq!(provider_status(&at_399), 502);

    let tool_at_400 = decode(r#"{"result":{"isError":true,"content":[]}}"#, 400).unwrap_err();
    assert_eq!(provider_status(&tool_at_400), 400);

    let text_yes = r#"{"result":{"content":[{"type":"text","text":"x"}],"isError":"no"}}"#;
    assert_eq!(provider_status(&decode(text_yes, 200).unwrap_err()), 502);

    let text_empty =
        r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"kept\"}]"}],"isError":""}}"#;
    assert_eq!(titles(&decode(text_empty, 200).unwrap()), vec!["kept"]);

    let null_flag = r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"kept\"}]"}],"isError":null}}"#;
    assert_eq!(titles(&decode(null_flag, 200).unwrap()), vec!["kept"]);

    let empty_list =
        r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"kept\"}]"}],"isError":[]}}"#;
    assert_eq!(titles(&decode(empty_list, 200).unwrap()), vec!["kept"]);

    let nonempty_list = r#"{"result":{"content":[{"type":"text","text":"x"}],"isError":[0]}}"#;
    assert_eq!(
        provider_status(&decode(nonempty_list, 200).unwrap_err()),
        502
    );

    let empty_object =
        r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"kept\"}]"}],"isError":{}}}"#;
    assert_eq!(titles(&decode(empty_object, 200).unwrap()), vec!["kept"]);

    let object_flag = r#"{"result":{"isError":{"reason":"x"},"content":[]}}"#;
    assert_eq!(provider_status(&decode(object_flag, 200).unwrap_err()), 502);

    let top_level = r#"{"result":7,"isError":true}"#;
    assert!(decode(top_level, 200).unwrap().results.is_empty());
}

fn mcp_tool_error() {
    let joined = r#"{"result":{"content":[{"type":"text","text":"first"},{"type":"text","text":"second"}],"isError":true}}"#;
    assert_eq!(
        provider_message(&decode(joined, 200).unwrap_err()),
        "AgentCore web search tool error: first second"
    );

    let with_empty = r#"{"result":{"content":[{"type":"text","text":"a"},{"type":"text","text":""}],"isError":true}}"#;
    assert_eq!(
        provider_message(&decode(with_empty, 200).unwrap_err()),
        "AgentCore web search tool error: a "
    );

    let only_empty = r#"{"result":{"content":[{"type":"text","text":""}],"isError":true}}"#;
    assert_eq!(
        provider_message(&decode(only_empty, 200).unwrap_err()),
        "AgentCore web search tool error: "
    );

    let unparsed =
        r#"{"result":{"content":[{"type":"text","text":"  not json  "}],"isError":true}}"#;
    assert_eq!(
        provider_message(&decode(unparsed, 200).unwrap_err()),
        "AgentCore web search tool error:   not json  "
    );

    let non_string = r#"{"result":{"content":[{"type":"text","text":7},{"type":"text","text":"kept"},{"type":"text","text":null}],"isError":true}}"#;
    assert_eq!(
        provider_message(&decode(non_string, 200).unwrap_err()),
        "AgentCore web search tool error: kept"
    );

    let no_blocks = r#"{"result":{"isError":true,"content":[{"type":"image","text":"x"}]}}"#;
    assert_eq!(
        provider_message(&decode(no_blocks, 200).unwrap_err()),
        "AgentCore web search tool error: {\"isError\":true,\"content\":[{\"type\":\"image\",\"text\":\"x\"}]}"
    );

    let long_text = "x".repeat(600);
    let long = format!(
        r#"{{"result":{{"isError":true,"content":[{{"type":"image","text":"{long_text}"}}]}}}}"#
    );
    let message = provider_message(&decode(&long, 200).unwrap_err()).to_owned();
    let detail = message
        .strip_prefix("AgentCore web search tool error: ")
        .unwrap();
    assert_eq!(detail.chars().count(), 500);
    assert!(detail.starts_with("{\"isError\":true,"));
}

fn agentcore_search_reply_error() {
    let truncated = r#"{"result":"#;
    let error = decode(truncated, 200).unwrap_err();
    assert!(matches!(&error, SearchReplyError::Json { .. }));
    assert_eq!(error.status(), None);

    let second_error = decode(truncated, 500).unwrap_err();
    assert!(matches!(&second_error, SearchReplyError::Json { .. }));
    assert_eq!(second_error.status(), None);

    let not_object = decode("7", 200).unwrap_err();
    assert!(matches!(&not_object, SearchReplyError::NotObject));
    assert_eq!(not_object.status(), None);

    let not_utf8 = decode_bytes(&[0xff, 0xfe], 200).unwrap_err();
    assert!(matches!(&not_utf8, SearchReplyError::NotUtf8 { .. }));
    assert_eq!(not_utf8.status(), None);

    for nonstandard in [
        r#"{"result":NaN}"#,
        r#"{"result":Infinity}"#,
        r#"{"result":"\ud800"}"#,
    ] {
        assert!(
            matches!(decode(nonstandard, 200), Err(SearchReplyError::Json { .. })),
            "{nonstandard}"
        );
    }
    let deep = format!("{}7{}", "[".repeat(200), "]".repeat(200));
    assert!(matches!(
        decode(&deep, 200),
        Err(SearchReplyError::Json { .. })
    ));

    let valid = r#"{"result":{"content":[{"type":"text","text":"[{\"title\":\"kept\"}]"}]}}"#;
    assert_eq!(titles(&decode(valid, 200).unwrap()), vec!["kept"]);
    assert_eq!(titles(&decode(valid, 500).unwrap()), vec!["kept"]);

    let mut bom = vec![0xef, 0xbb, 0xbf];
    bom.extend_from_slice(valid.as_bytes());
    assert_eq!(titles(&decode_bytes(&bom, 200).unwrap()), vec!["kept"]);

    let utf16 = [0xff, 0xfe, 0x7b, 0x00, 0x7d, 0x00];
    assert!(matches!(
        decode_bytes(&utf16, 200),
        Err(SearchReplyError::NotUtf8 { .. })
    ));
}
