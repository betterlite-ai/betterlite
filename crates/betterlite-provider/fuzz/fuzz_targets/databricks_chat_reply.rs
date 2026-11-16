#![no_main]

use std::collections::BTreeMap;

use betterlite_provider::databricks_chat::response::{ResponseInput, transform_response};
use betterlite_provider::databricks_chat::stream::ChunkParser;
use betterlite_provider::ollama_chat::ids::RandomIds;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    for json_mode in [false, true] {
        let _reply = transform_response(ResponseInput {
            body: text,
            status_code: 200,
            headers: BTreeMap::new(),
            custom_llm_provider: None,
            json_mode,
            ids: &RandomIds,
        });
        let _chunk = ChunkParser {
            json_mode,
            last_function_name: None,
        }
        .parse(text, &RandomIds);
    }
});
