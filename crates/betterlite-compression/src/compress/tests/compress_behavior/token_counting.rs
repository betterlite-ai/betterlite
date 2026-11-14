use super::super::*;

#[tokio::test]
async fn counts_the_original_and_compressed_messages() {
    let messages = vec![
        json!({"role": "system", "content": "be brief"}),
        json!({"role": "user", "content": "one two three"}),
        json!({"role": "assistant", "content": "four five"}),
        json!({"role": "user", "content": "six seven"}),
    ];
    let counter = fixture_counter(vec![100, 40]);
    let result = compress(&request(&messages, 0, Some(0)), &counter)
        .await
        .expect("compressed");
    assert_eq!(result.original_tokens, 100);
    assert_eq!(result.compressed_tokens, 40);
    let calls = counter.calls.lock().expect("recorded calls").clone();
    let message_calls: Vec<CountCall> = calls
        .into_iter()
        .filter(|call| matches!(call, CountCall::Messages { .. }))
        .collect();
    assert_eq!(
        message_calls,
        vec![
            CountCall::Messages {
                model: MODEL.to_owned(),
                messages: messages.clone(),
            },
            CountCall::Messages {
                model: MODEL.to_owned(),
                messages: result.messages,
            },
        ]
    );
}
