#![no_main]

use betterlite_server::guardrails::bedrock_passthrough::event_stream::frames::{
    parse_event_stream, rebuild_event_stream,
};
use betterlite_server::guardrails::bedrock_passthrough::event_stream::groups::{
    group_texts, redistribute,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    let parsed = parse_event_stream(body);
    let groups = group_texts(&parsed.frames);
    let returned: Vec<String> = groups
        .groups
        .iter()
        .map(|group| group.fragments.concat().to_uppercase())
        .collect();
    let Ok(new_texts) = redistribute(&groups, &returned) else {
        return;
    };
    let _rebuilt = rebuild_event_stream(&parsed, &new_texts);
});
