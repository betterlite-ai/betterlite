#![no_main]

use betterlite_logging::arize::phoenix::parsing::parse_prompt_data;
use betterlite_logging::arize::phoenix::records::PromptVersionEnvelope;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    if let Ok(envelope) = serde_json::from_slice::<PromptVersionEnvelope>(body)
        && let Some(data) = envelope.data
    {
        let _template = parse_prompt_data(data, "fuzz");
    }
});
