#![no_main]

use std::convert::Infallible;
use std::str;

use betterlite_anthropic::stream_state::state::UuidV4;
use betterlite_anthropic::usage::ReasoningTokenCounter;
use betterlite_provider::snowflake::embedding::transform_embedding_response;
use betterlite_provider::snowflake::response::{
    PostCall, PostCallLogger, ResponseInput, transform_response_claude, transform_response_openai,
};
use betterlite_provider::snowflake::stream::{
    SnowflakeStreamingHandler, get_model_response_iterator,
};
use libfuzzer_sys::fuzz_target;

struct Words;

impl ReasoningTokenCounter for Words {
    type Failure = Infallible;

    fn count_reasoning_tokens(&self, text: &str) -> Result<usize, Infallible> {
        Ok(text.split_whitespace().count())
    }
}

struct Silent;

impl PostCallLogger for Silent {
    fn post_call(&self, _call: PostCall<'_>) {}
}

fuzz_target!(|bytes: &[u8]| {
    let Ok(body) = str::from_utf8(bytes) else {
        return;
    };
    let Ok(request) = serde_json::from_str("{}") else {
        return;
    };
    let messages = Vec::new();
    let input = ResponseInput {
        model: "snowflake/claude-3-5-sonnet",
        raw_response: body,
        request_data: &request,
        messages: &messages,
    };
    let _openai = transform_response_openai(&input, &Silent);
    let _claude = transform_response_claude(&input, &Silent, &Words);
    let _embedding = transform_embedding_response("snowflake/model", body);
    let mut handler: SnowflakeStreamingHandler<(), (), Words> =
        get_model_response_iterator((), false, None, Words, &UuidV4);
    if let Ok(chunk) = serde_json::from_str(body) {
        let _parsed = handler.chunk_parser(&chunk);
    }
});
