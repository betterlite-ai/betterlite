#![no_main]

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use betterlite_server::guardrails::anthropic_translation::stream_rewrite::line::rewrite_sse_events;
use betterlite_server::guardrails::anthropic_translation::stream_rewrite::rewriter::EventRewriter;
use betterlite_server::guardrails::tool_call_identity::ToolCallShape;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let mut text = EventRewriter::TextDelta {
        replacements: VecDeque::from(["rewritten".to_owned()]),
    };
    let _text_pass = rewrite_sse_events(input, &mut text);
    let mut tools = EventRewriter::ToolUse {
        rewrites_by_block: BTreeMap::from([(
            0_i64,
            ToolCallShape {
                name: Some("name".to_owned()),
                arguments: "{}".to_owned(),
            },
        )]),
        arguments_sent: BTreeSet::new(),
    };
    let _tool_pass = rewrite_sse_events(input, &mut tools);
});
