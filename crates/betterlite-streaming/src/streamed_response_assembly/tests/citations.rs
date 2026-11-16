use super::{object, response_of};

const FIRST_CHUNK: &str = r#"{"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o", "choices": [{"index": 0, "delta": {"content": "A", "provider_specific_fields": "#;

// Each case is the provider_specific_fields of the first chunk, the later chunks and the combined map.
const CASES: [(&str, &str, Option<&str>); 6] = [
    (
        r#"{"citation": ["c1", "c2"]}"#,
        r#"{"choices": [{"index": 0, "delta": {"provider_specific_fields": {"citation": ["c3"], "web_search_results": ["t1"]}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields": {"web_search_results": ["t2"], "citation_count": 3}}}]}"#,
        Some(
            r#"{"citations": [["c1", "c2"], ["c3"]], "web_search_results": ["t2"], "citation_count": 3}"#,
        ),
    ),
    (
        r#"{"citation": "c1"}"#,
        r#"{"choices": [{"index": 0, "delta": {"provider_specific_fields": {"citation": "c2"}}}]}"#,
        Some(r#"{"citations": [["c1", "c2"]]}"#),
    ),
    (
        r#"{"citation": ["g1"]}"#,
        r#"{"choices": [{"index": 0, "delta": {"provider_specific_fields": {"citations": ["mine"]}}}]}"#,
        Some(r#"{"citations": ["mine"]}"#),
    ),
    (
        r#""nope""#,
        r#"{"choices": [{"index": 0, "delta": {"provider_specific_fields": {"k": 1}}}]}"#,
        Some(r#"{"k": 1}"#),
    ),
    ("{}", r#"{"choices": []}"#, None),
    (
        r#"{"citation": null, "warning": "w"}"#,
        r#"{"choices": [{"index": 0, "delta": {"provider_specific_fields": {"citation": ["c9"], "web_search_results": ["t1"]}}}]}"#,
        Some(r#"{"citations": [["c9"]], "warning": "w", "web_search_results": ["t1"]}"#),
    ),
];

pub(super) async fn provider_fields_citations() {
    for (first_fields, later_chunks, expected) in CASES {
        let source = format!("[{FIRST_CHUNK}{first_fields}}}}}]}}, {later_chunks}]");
        let response = response_of(&source, 1).await;
        assert_eq!(
            response.choices[0].message.provider_specific_fields,
            expected.map(object),
            "{first_fields}"
        );
    }
}
