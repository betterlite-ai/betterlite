#![no_main]

use betterlite_guardrails::model_armor::file_scanning::plan_file_scans;
use betterlite_prompts::message_content::Message;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(messages) = serde_json::from_slice::<Vec<Message>>(bytes) else {
        return;
    };
    drop(plan_file_scans(&messages));
});
