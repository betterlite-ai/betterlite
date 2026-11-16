#![no_main]

use std::collections::BTreeMap;

use betterlite_provider::databricks_chat::response::{ResponseInput, transform_response};
use betterlite_provider::ollama_chat::ids::RandomIds;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(body) = std::str::from_utf8(bytes) else {
        return;
    };
    for json_mode in [false, true] {
        let _reply = transform_response(ResponseInput {
            body,
            status_code: 200,
            headers: BTreeMap::new(),
            custom_llm_provider: None,
            json_mode: Some(json_mode),
            ids: &RandomIds,
        });
    }
});
