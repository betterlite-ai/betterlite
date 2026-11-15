use super::support::chunks;
use super::{Harness, chat_of, object};

pub(super) async fn provider_fields_citations() {
    provider_fields_merge().await;
    provider_fields_non_list_citations().await;
    provider_fields_explicit_override().await;
    provider_fields_skip_non_dict().await;
    provider_fields_empty_map().await;
    provider_fields_null_citation().await;
}

pub(super) async fn provider_fields_merge() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "provider_specific_fields": {"citation": ["c1", "c2"]}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields":
            {"citation": ["c3"], "web_search_results": ["t1"]}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields":
            {"web_search_results": ["t2"], "citation_count": 3}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected = object(
        r#"{"citations": [["c1", "c2"], ["c3"]], "web_search_results": ["t2"], "citation_count": 3}"#,
    );
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn provider_fields_non_list_citations() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "provider_specific_fields": {"citation": "c1"}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields": {"citation": "c2"}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected = object(r#"{"citations": [["c1", "c2"]]}"#);
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn provider_fields_explicit_override() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "provider_specific_fields": {"citation": ["g1"]}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields": {"citations": ["mine"]}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected = object(r#"{"citations": ["mine"]}"#);
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn provider_fields_skip_non_dict() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A", "provider_specific_fields": "nope"}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields": {"k": 1}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected = object(r#"{"k": 1}"#);
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}

pub(super) async fn provider_fields_empty_map() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A", "provider_specific_fields": {}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    assert_eq!(response.choices[0].message.provider_specific_fields, None);
}

pub(super) async fn provider_fields_null_citation() {
    let stream = chunks(
        r#"[
        {"id": "chatcmpl-1", "object": "chat.completion.chunk", "created": 1700000000, "model": "gpt-4o",
         "choices": [{"index": 0, "delta": {"content": "A",
            "provider_specific_fields": {"citation": null, "warning": "w"}}}]},
        {"choices": [{"index": 0, "delta": {"provider_specific_fields":
            {"citation": ["c9"], "web_search_results": ["t1"]}}}]}
    ]"#,
    );
    let mut harness = Harness::new(Some(stream));
    harness.messages = true;
    harness.counter.response_text = 1;
    let response = chat_of(&harness).await;
    let expected =
        object(r#"{"citations": [["c9"]], "warning": "w", "web_search_results": ["t1"]}"#);
    assert_eq!(
        response.choices[0].message.provider_specific_fields,
        Some(expected)
    );
}
