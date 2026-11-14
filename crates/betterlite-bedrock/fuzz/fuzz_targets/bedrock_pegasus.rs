#![no_main]

use async_trait::async_trait;
use betterlite_bedrock::pegasus::counter::TokenCounter;
use betterlite_bedrock::pegasus::reply::{ReplyContext, decode_pegasus_reply};
use betterlite_bedrock::pegasus::response::{
    PegasusChoice, PegasusMessage, PegasusResponse, PegasusUsage,
};
use betterlite_tokenizer::count::error::CountError;
use betterlite_tokenizer::count::request::Message;
use libfuzzer_sys::fuzz_target;

struct ZeroCounter;

#[async_trait]
impl TokenCounter for ZeroCounter {
    async fn count_messages(&self, _messages: &[Message]) -> Result<i64, CountError> {
        Ok(0)
    }

    async fn count_text(&self, _text: &str) -> Result<i64, CountError> {
        Ok(0)
    }
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
        return;
    };
    let headers = [
        (
            "x-amzn-bedrock-input-token-count".to_owned(),
            "1".to_owned(),
        ),
        (
            "x-amzn-bedrock-output-token-count".to_owned(),
            "1".to_owned(),
        ),
    ];
    let messages: [Message; 0] = [];
    let mut response = PegasusResponse {
        id: "fuzz".to_owned(),
        choices: vec![PegasusChoice {
            index: 0,
            message: PegasusMessage {
                role: "assistant".to_owned(),
                content: None,
                tool_calls: None,
            },
            finish_reason: None,
        }],
        created: 0,
        model: String::new(),
        usage: PegasusUsage::default(),
    };
    let context = ReplyContext {
        status: 200,
        headers: &headers,
        model: "twelvelabs.pegasus-1-2-v1:0",
        messages: &messages,
        now: 0.0,
    };
    let _outcome = runtime.block_on(decode_pegasus_reply(
        bytes,
        &context,
        &mut response,
        &ZeroCounter,
    ));
});
