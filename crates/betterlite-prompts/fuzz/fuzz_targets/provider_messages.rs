#![no_main]

use betterlite_prompts::azure::convert::to_azure;
use betterlite_prompts::azure::stringify::stringify_tool_content;
use betterlite_prompts::azure::text::azure_text_pt;
use betterlite_prompts::cohere::history::cohere_messages_pt_v2;
use betterlite_prompts::cohere::prompt::cohere_message_pt;
use betterlite_prompts::message_content::Message;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(messages) = serde_json::from_slice::<Vec<Message>>(bytes) else {
        return;
    };
    drop(to_azure(&messages));
    drop(stringify_tool_content(&messages));
    drop(azure_text_pt(&messages));
    drop(cohere_messages_pt_v2(&messages));
    drop(cohere_message_pt(&messages));
});
