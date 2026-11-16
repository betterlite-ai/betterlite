#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;

use crate::databricks_chat::response::{ResponseInput, transform_response};
use crate::databricks_chat::stream::ChunkParser;
use crate::ollama_chat::ids::RandomIds;

// Every corpus file goes through the reply decoder and the chunk parser, in and out of json mode
#[test]
fn replays_the_databricks_chat_reply_corpus() {
    let outcomes: Vec<(bool, bool)> = fs::read_dir("fuzz/corpus/databricks_chat_reply")
        .expect("the reply corpus exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter_map(|path| fs::read(path).ok())
        .filter_map(|bytes| String::from_utf8(bytes).ok())
        .flat_map(|text| [false, true].map(|json_mode| (text.clone(), json_mode)))
        .map(|(text, json_mode)| {
            let reply = transform_response(ResponseInput {
                body: &text,
                status_code: 200,
                headers: BTreeMap::new(),
                custom_llm_provider: None,
                json_mode,
                ids: &RandomIds,
            });
            let chunk = ChunkParser {
                json_mode,
                last_function_name: None,
            }
            .parse(&text, &RandomIds);
            (reply.is_ok(), chunk.is_ok())
        })
        .collect();
    assert!(outcomes.iter().any(|(reply, _)| *reply));
    assert!(outcomes.iter().any(|(_, chunk)| *chunk));
}
