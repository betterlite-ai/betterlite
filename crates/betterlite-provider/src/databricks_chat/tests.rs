#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;

use crate::databricks_chat::response::{ResponseInput, transform_response};
use crate::databricks_chat::stream::ChunkParser;
use crate::ollama_chat::ids::RandomIds;

// Every corpus file goes through the reply decoder and the chunk parser, in and out of json mode
#[test]
fn replays_the_databricks_chat_reply_corpus() {
    let mut replies = 0_usize;
    let mut chunks = 0_usize;
    for entry in fs::read_dir("fuzz/corpus/databricks_chat_reply").expect("the reply corpus exists")
    {
        let path = entry.expect("the corpus entry reads").path();
        let bytes = fs::read(&path).expect("the corpus file reads");
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };
        for json_mode in [false, true] {
            let reply = transform_response(ResponseInput {
                body: text,
                status_code: 200,
                headers: BTreeMap::new(),
                custom_llm_provider: None,
                json_mode,
                ids: &RandomIds,
            });
            replies = replies.saturating_add(usize::from(reply.is_ok()));
            let chunk = ChunkParser::new(json_mode).parse(text, &RandomIds);
            chunks = chunks.saturating_add(usize::from(chunk.is_ok()));
        }
    }
    assert!(replies > 0);
    assert!(chunks > 0);
}
