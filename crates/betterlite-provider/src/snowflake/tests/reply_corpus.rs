use std::fs;
use std::str;

use betterlite_prompts::arguments::json::Json;

use super::support::{SequenceIds, WordCounter, parsed};
use crate::snowflake::embedding::transform_embedding_response;
use crate::snowflake::response::{
    PostCall, PostCallLogger, ResponseInput, transform_response_claude, transform_response_openai,
};
use crate::snowflake::stream::{SnowflakeStreamingHandler, get_model_response_iterator};

struct Silent;

impl PostCallLogger for Silent {
    fn post_call(&self, _call: PostCall<'_>) {}
}

#[test]
fn replays_the_snowflake_reply_corpus() {
    let ids = SequenceIds::default();
    let mut handler: SnowflakeStreamingHandler<(), (), WordCounter> =
        get_model_response_iterator((), false, None, WordCounter, &ids);
    let request = parsed("{}");
    let turns: Vec<Json> = Vec::new();
    let seen = fs::read_dir("fuzz/corpus/snowflake_reply")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).unwrap();
            let Ok(body) = str::from_utf8(&bytes) else {
                return;
            };
            let input = ResponseInput {
                model: "snowflake/claude-3-5-sonnet",
                raw_response: body,
                request_data: &request,
                messages: &turns,
            };
            let _openai = transform_response_openai(&input, &Silent);
            let _claude = transform_response_claude(&input, &Silent, &WordCounter);
            let _embedding = transform_embedding_response("snowflake/model", body);
            if let Ok(chunk) = serde_json::from_str::<Json>(body) {
                let _parsed = handler.chunk_parser(&chunk);
            }
        })
        .count();
    assert!(seen >= 10_usize, "the corpus holds {seen} files");
}
