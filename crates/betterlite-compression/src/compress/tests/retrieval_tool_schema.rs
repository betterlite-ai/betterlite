use super::*;

#[tokio::test]
async fn lists_every_cache_key_in_message_order() {
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "first"}),
        json!({"role": "user", "content": "second"}),
        json!({"role": "user", "content": "last question"}),
    ];
    let counter = fixture_counter(vec![40, 20]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.tools.len(), 1);
    assert_eq!(result.tools[0]["type"], "function");
    assert_eq!(
        result.tools[0]["function"]["name"],
        crate::retrieval_tool::RETRIEVAL_TOOL_NAME
    );
    let keys = &result.tools[0]["function"]["parameters"]["properties"]["key"]["enum"];
    assert_eq!(keys, &json!(["message_1", "message_2"]));
}

#[tokio::test]
async fn builds_no_tool_when_nothing_is_dropped() {
    let messages = vec![
        json!({"role": "system", "content": "sys"}),
        json!({"role": "user", "content": "last question"}),
    ];
    let counter = fixture_counter(vec![40, 20]);
    let result = compress(&request(&messages, 0, Some(1000)), &counter)
        .await
        .expect("compressed");
    assert!(result.cache.is_empty());
    assert!(result.tools.is_empty());
}
