use std::fs;
use std::path::Path;

use betterlite_config::sections::guardrail_identity::item::GuardrailValue;

use crate::cato::wire::AnalyzeResponse;

#[test]
fn cato_guardrail_parses_the_redacted_chat_answer() {
    let answer: AnalyzeResponse = serde_json::from_str(
        r#"{"redacted_chat":{"all_redacted_messages":[{"content":"REDACTED"},{"content":null},{}]}}"#,
    )
    .expect("answer");
    let messages = answer
        .redacted_chat
        .and_then(|chat| chat.all_redacted_messages)
        .expect("redacted messages");
    assert_eq!(messages.len(), 3);
    assert_eq!(
        messages[0].content,
        Some(GuardrailValue::Text("REDACTED".to_owned()))
    );
    assert_eq!(messages[1].content, None);
    assert_eq!(messages[2].content, None);
}

#[test]
fn cato_guardrail_reads_an_answer_without_a_redacted_chat() {
    let answer: AnalyzeResponse = serde_json::from_str(r#"{"redacted_chat":{}}"#).expect("answer");
    let chat = answer.redacted_chat.expect("chat");
    assert!(chat.all_redacted_messages.is_none());
}

#[test]
fn cato_guardrail_replays_the_cato_response_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/cato_response");
    let paths: Vec<_> = fs::read_dir(corpus).unwrap().collect();
    assert!(!paths.is_empty());
    assert!(paths.into_iter().all(|entry| {
        let bytes = fs::read(entry.unwrap().path()).unwrap();
        serde_json::from_slice::<AnalyzeResponse>(&bytes).is_ok()
    }));
}
