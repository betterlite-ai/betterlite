use std::sync::{Mutex, PoisonError};

use betterlite_models::decisions::Json;
use betterlite_tokenizer::count::request::Message;

use super::support::{FixedCounter, InfoLimits, message, spec};
use crate::static_eligibility::tokens::{
    TokenCountError, TokenCounter, TokenPayload, count_for_pre_call_checks, count_request_tokens,
};

const MODEL: &str = "gpt";
const WINDOW: &str = r#"{"max_input_tokens":100}"#;

#[tokio::test]
async fn count_request_tokens_prefixes_the_anthropic_system_block() {
    let counter = FixedCounter { count: 0 };
    let messages: Vec<Json> = vec![message("hello")];
    let payload = TokenPayload {
        messages: Some(&messages),
        input: None,
        instructions: None,
        system: Some(&Json::Text("be brief".to_owned())),
        tools: &[],
    };
    assert_eq!(count_request_tokens(&payload, &counter).await.unwrap(), 2);
    let empty = TokenPayload {
        messages: Some(&messages),
        input: None,
        instructions: None,
        system: Some(&Json::Text(String::new())),
        tools: &[],
    };
    assert_eq!(count_request_tokens(&empty, &counter).await.unwrap(), 1);
}

#[tokio::test]
async fn count_request_tokens_rejects_a_request_without_messages() {
    let counter = FixedCounter { count: 0 };
    let payload = TokenPayload {
        messages: None,
        input: None,
        instructions: None,
        system: None,
        tools: &[],
    };
    assert!(matches!(
        count_request_tokens(&payload, &counter).await,
        Err(TokenCountError::NoInput)
    ));
}

#[derive(Default)]
struct RecordingCounter {
    counted: Mutex<Vec<Message>>,
}

#[async_trait::async_trait]
impl TokenCounter for RecordingCounter {
    async fn count_messages(
        &self,
        messages: &[Message],
        _tools: &[Json],
    ) -> Result<usize, TokenCountError> {
        self.counted
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .extend_from_slice(messages);
        Ok(messages.len())
    }
}

#[tokio::test]
async fn count_request_tokens_converts_a_responses_input() {
    let counter = FixedCounter { count: 0 };
    let text = Json::Text("hello world".to_owned());
    let string_payload = TokenPayload {
        messages: None,
        input: Some(&text),
        instructions: Some("be brief"),
        system: None,
        tools: &[],
    };
    assert_eq!(
        count_request_tokens(&string_payload, &counter)
            .await
            .unwrap(),
        2
    );
    let items: Json = serde_json::from_str(r#"[{"role":"user","content":"hello world"}]"#).unwrap();
    let list_payload = TokenPayload {
        messages: None,
        input: Some(&items),
        instructions: None,
        system: None,
        tools: &[],
    };
    assert_eq!(
        count_request_tokens(&list_payload, &counter).await.unwrap(),
        1
    );
}

#[tokio::test]
async fn count_request_tokens_prefers_the_chat_messages_over_the_input() {
    let counter = FixedCounter { count: 0 };
    let messages: Vec<Json> = vec![message("hello")];
    let input = Json::Text("ignored".to_owned());
    let payload = TokenPayload {
        messages: Some(&messages),
        input: Some(&input),
        instructions: Some("ignored"),
        system: None,
        tools: &[],
    };
    assert_eq!(count_request_tokens(&payload, &counter).await.unwrap(), 1);
}

#[tokio::test]
async fn count_request_tokens_inspects_a_reasoning_item_without_replaying_it() {
    let counter = RecordingCounter::default();
    let input: Json = serde_json::from_str(
        r#"[{"type":"reasoning","content":[{"type":"reasoning_text","text":"think"}]}]"#,
    )
    .unwrap();
    let payload = TokenPayload {
        messages: None,
        input: Some(&input),
        instructions: None,
        system: None,
        tools: &[],
    };
    assert_eq!(count_request_tokens(&payload, &counter).await.unwrap(), 1);
    let recorded = counter
        .counted
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded.first().map(|message| message.role.as_str()),
        Some("user")
    );
    assert!(
        recorded
            .first()
            .is_some_and(|message| message.reasoning_content.is_none())
    );
    drop(recorded);
}

#[tokio::test]
async fn count_for_pre_call_checks_counts_a_responses_input() {
    let counter = FixedCounter { count: 7 };
    let input = Json::Text("hello".to_owned());
    let payload = TokenPayload {
        messages: None,
        input: Some(&input),
        instructions: None,
        system: None,
        tools: &[],
    };
    let windowed = vec![spec("gpt", "d1", "openai/gpt-4o", WINDOW)];
    assert_eq!(
        count_for_pre_call_checks(MODEL, &windowed, &payload, &InfoLimits, &counter).await,
        Some(8)
    );
}

#[tokio::test]
async fn count_for_pre_call_checks_needs_messages_and_a_declared_window() {
    let counter = FixedCounter { count: 7 };
    let messages: Vec<Json> = vec![message("hello")];
    let with_messages = TokenPayload {
        messages: Some(&messages),
        input: None,
        instructions: None,
        system: None,
        tools: &[],
    };
    let windowed = vec![spec("gpt", "d1", "openai/gpt-4o", WINDOW)];
    assert_eq!(
        count_for_pre_call_checks(MODEL, &windowed, &with_messages, &InfoLimits, &counter).await,
        Some(8)
    );
    let open = vec![spec("gpt", "d1", "openai/gpt-4o", "")];
    assert_eq!(
        count_for_pre_call_checks(MODEL, &open, &with_messages, &InfoLimits, &counter).await,
        None
    );
    assert_eq!(
        count_for_pre_call_checks(
            MODEL,
            &windowed,
            &TokenPayload {
                messages: None,
                input: None,
                instructions: None,
                system: None,
                tools: &[]
            },
            &InfoLimits,
            &counter
        )
        .await,
        None
    );
}

#[tokio::test]
async fn count_for_pre_call_checks_swallows_a_counter_failure() {
    struct Failing;
    #[async_trait::async_trait]
    impl TokenCounter for Failing {
        async fn count_messages(
            &self,
            _messages: &[Message],
            _tools: &[Json],
        ) -> Result<usize, TokenCountError> {
            Err(TokenCountError::NoInput)
        }
    }
    let messages: Vec<Json> = vec![message("hello")];
    let payload = TokenPayload {
        messages: Some(&messages),
        input: None,
        instructions: None,
        system: None,
        tools: &[],
    };
    let windowed = vec![spec("gpt", "d1", "openai/gpt-4o", WINDOW)];
    assert_eq!(
        count_for_pre_call_checks(MODEL, &windowed, &payload, &InfoLimits, &Failing).await,
        None
    );
}

#[tokio::test]
async fn count_request_tokens_counts_a_system_block_list_once() {
    let counter = FixedCounter { count: 0 };
    let messages: Vec<Json> = vec![message("hello")];
    let blocks: Json = serde_json::from_str(r#"[{"type":"text","text":"be brief"}]"#).unwrap();
    let listed = TokenPayload {
        messages: Some(&messages),
        input: None,
        instructions: None,
        system: Some(&blocks),
        tools: &[],
    };
    assert_eq!(count_request_tokens(&listed, &counter).await.unwrap(), 2);
    let none = Json::List(Vec::new());
    let empty = TokenPayload {
        messages: Some(&messages),
        input: None,
        instructions: None,
        system: Some(&none),
        tools: &[],
    };
    assert_eq!(count_request_tokens(&empty, &counter).await.unwrap(), 1);
}
