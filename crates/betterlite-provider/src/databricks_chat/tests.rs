#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;

use crate::databricks_chat::response::{ResponseInput, transform_response};
use crate::ollama_chat::ids::RandomIds;

// Every seeded Databricks reply body goes through the transform without panicking
#[test]
fn replays_the_databricks_chat_reply_corpus() {
    let seen = fs::read_dir("fuzz/corpus/databricks_chat_reply")
        .expect("the reply corpus exists")
        .map(|entry| entry.expect("the corpus entry reads").path())
        .filter(|path| path.is_file())
        .inspect(|path| {
            let bytes = fs::read(path).expect("the corpus body reads");
            let Ok(body) = std::str::from_utf8(&bytes) else {
                return;
            };
            let _reply = transform_response(ResponseInput {
                body,
                status_code: 200,
                headers: BTreeMap::new(),
                custom_llm_provider: None,
                json_mode: Some(false),
                ids: &RandomIds,
            });
        })
        .count();
    assert!(seen > 0_usize);
}
