use super::super::*;

#[tokio::test]
async fn keeps_system_last_user_and_last_assistant() {
    let messages = vec![
        json!({"role": "system", "content": "system instructions"}),
        json!({"role": "user", "content": "old question"}),
        json!({"role": "assistant", "content": "old answer"}),
        json!({"role": "user", "content": "new question"}),
    ];
    let counter = fixture_counter(vec![80, 60]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages.len(), 4);
    assert_eq!(result.messages[0], messages[0]);
    assert_eq!(result.messages[2], messages[2]);
    assert_eq!(result.messages[3], messages[3]);
    assert!(
        result.messages[1]["content"]
            .as_str()
            .is_some_and(|text| text.starts_with("[Compressed:"))
    );
    assert_eq!(
        result.cache.get("message_1").map(String::as_str),
        Some("old question")
    );
}

#[tokio::test]
async fn keeps_every_message_up_to_the_last_cache_control() {
    let messages = vec![
        json!({"role": "system", "content": "system instructions"}),
        json!({"role": "user", "content": "pinned question"}),
        json!({"role": "assistant", "content": "pinned answer"}),
        json!({
            "role": "user",
            "content": [
                {"type": "text", "text": "breakpoint question",
                 "cache_control": {"type": "ephemeral"}}
            ],
        }),
        json!({"role": "user", "content": "middle question"}),
        json!({"role": "user", "content": "new question"}),
    ];
    let counter = fixture_counter(vec![80, 60]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.messages.len(), 6);
    assert_eq!(result.messages[0], messages[0]);
    assert_eq!(result.messages[1], messages[1]);
    assert_eq!(result.messages[2], messages[2]);
    assert_eq!(result.messages[3], messages[3]);
    assert_eq!(result.messages[5], messages[5]);
    assert!(
        result.messages[4]["content"]
            .as_str()
            .is_some_and(|text| text.starts_with("[Compressed:"))
    );
    assert_eq!(
        result.cache.get("message_4").map(String::as_str),
        Some("middle question")
    );
}
