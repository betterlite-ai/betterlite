#![no_main]

use betterlite_prompts::file_ids::managed_id::get_format_from_file_id;
use betterlite_prompts::file_ids::mapping::FileIdMapping;
use betterlite_prompts::file_ids::messages::{
    get_file_ids_from_messages, update_messages_with_file_ids,
};
use betterlite_prompts::file_ids::responses::{ResponsesInput, update_responses_file_ids};
use betterlite_prompts::file_ids::tools::{
    ResponsesTool, decode_vector_store_ids, update_tools_with_file_ids,
};
use betterlite_prompts::message_content::Message;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 0x0001_0000 {
        return;
    }
    if let Ok(text) = str::from_utf8(bytes) {
        drop(get_format_from_file_id(Some(text)));
    }
    let mapping = FileIdMapping::default();
    if let Ok(parsed) = serde_json::from_slice::<Vec<Message>>(bytes) {
        let mut messages = parsed;
        update_messages_with_file_ids(&mut messages, Some("deployment-1"), &mapping);
        drop(get_file_ids_from_messages(&messages));
    }
    if let Ok(input) = serde_json::from_slice::<ResponsesInput>(bytes) {
        drop(update_responses_file_ids(
            &input,
            Some("deployment-1"),
            &mapping,
        ));
    }
    if let Ok(tools) = serde_json::from_slice::<Option<Vec<ResponsesTool>>>(bytes) {
        drop(decode_vector_store_ids(tools.as_deref()));
        drop(update_tools_with_file_ids(
            tools.as_deref(),
            Some("deployment-1"),
            &mapping,
        ));
    }
});
