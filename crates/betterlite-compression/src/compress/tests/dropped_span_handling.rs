use super::*;

#[test]
fn marks_every_index_of_a_dropped_span() {
    let kept = BTreeSet::from([0, 3]);
    let spans = vec![vec![1_usize, 2], vec![4, 5]];
    assert_eq!(
        get_dropped_tool_span_indices(&kept, &spans),
        BTreeSet::from([1, 2, 4, 5])
    );
    let partially_kept = BTreeSet::from([1]);
    assert!(get_dropped_tool_span_indices(&partially_kept, &spans[..1]).is_empty());
}

#[tokio::test]
async fn omits_the_dropped_exchange_and_stubs_the_rest() {
    let messages = vec![
        json!({"role": "user", "content": "question one"}),
        json!({"role": "assistant", "content": [
            {"type": "tool_use", "id": "t1", "name": "lookup", "input": {}}
        ]}),
        json!({"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "t1", "content": [{"type": "text", "text": "result"}]}
        ]}),
        json!({"role": "assistant", "content": "middle answer"}),
        json!({"role": "user", "content": "question two"}),
    ];
    let counter = fixture_counter(vec![90, 20]);
    let mut compress_request = request(&messages, 0, Some(0));
    compress_request.call_type = "anthropic_messages";
    let result = compress(&compress_request, &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages.len(), 3);
    assert!(
        result.messages[0]["content"]
            .as_str()
            .is_some_and(|text| text.starts_with("[Compressed:"))
    );
    assert_eq!(result.messages[1], messages[3]);
    assert_eq!(result.messages[2], messages[4]);
    assert_eq!(
        result.cache,
        IndexMap::from([("message_0".to_owned(), "question one".to_owned())])
    );
    let keys = &result.tools[0]["input_schema"]["properties"]["key"]["enum"];
    assert_eq!(keys, &json!(["message_0"]));
}
