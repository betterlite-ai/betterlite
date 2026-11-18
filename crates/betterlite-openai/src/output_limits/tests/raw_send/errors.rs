#![cfg(test)]

use std::error::Error as _;

use async_trait::async_trait;
use betterlite_tokenizer::count::error::CountError;
use http::HeaderMap;
use rstest::rstest;
use serde_json::{Value, json};

use crate::output_limits::tokens::{PromptCountError, PromptTokens};

use super::{
    BadRequestReply, OUTPUT_TOKEN_LIMIT_MARKER, RawChatError, ScriptedTransport, fields, fixed_now,
    recovery, send_raw_chat_async, send_raw_chat_sync,
};

struct FailedCount;

#[async_trait]
impl PromptTokens for FailedCount {
    async fn count(&self, _model: &str, _messages: &[Value]) -> Result<usize, PromptCountError> {
        Err(PromptCountError::Count {
            source: CountError::Overflow,
        })
    }
}

#[rstest]
#[case::sync(false)]
#[case::asynchronous(true)]
#[tokio::test]
async fn recovery_preserves_count_source(#[case] asynchronous: bool) {
    let transport =
        ScriptedTransport::from(Err(RawChatError::BadRequest(Box::new(BadRequestReply {
            message: OUTPUT_TOKEN_LIMIT_MARKER.to_owned(),
            headers: HeaderMap::new(),
            body: Value::Null,
        }))));
    let data = fields(&[("model", json!("gpt-3.5-turbo")), ("messages", json!([]))]);
    let error = if asynchronous {
        send_raw_chat_async(&transport, &data, 1.0, fixed_now, &recovery(&FailedCount)).await
    } else {
        send_raw_chat_sync(&transport, &data, 1.0, &recovery(&FailedCount)).await
    }
    .expect_err("counting must fail");
    assert!(matches!(error, RawChatError::Count { .. }));
    let source = error
        .source()
        .unwrap()
        .downcast_ref::<Box<PromptCountError>>()
        .unwrap();
    assert!(matches!(
        source.as_ref(),
        PromptCountError::Count {
            source: CountError::Overflow
        }
    ));
    assert!(matches!(
        source.source().unwrap().downcast_ref::<CountError>(),
        Some(CountError::Overflow)
    ));
}

#[test]
fn encode_error_preserves_json_source() {
    let source = serde_json::from_str::<Value>("{").unwrap_err();
    let error = RawChatError::Encode { source };
    let cause = error
        .source()
        .unwrap()
        .downcast_ref::<serde_json::Error>()
        .unwrap();
    assert!(cause.is_eof());
    assert_eq!(cause.line(), 1);
    assert_eq!(cause.column(), 1);
}
