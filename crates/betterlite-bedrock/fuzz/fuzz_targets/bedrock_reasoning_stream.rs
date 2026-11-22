#![no_main]

use std::str::from_utf8;

use betterlite_bedrock::reasoning_stream::{ReasoningTagState, decode_reasoning_delta};
use libfuzzer_sys::fuzz_target;

// Raw invalid UTF-8 is the transport decoder's error, so only the fragments it would hand on are exercised here.
fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = from_utf8(bytes) else {
        return;
    };
    let mut state = ReasoningTagState::default();
    let mut rest = text;
    let mut width = 1_usize;
    while !rest.is_empty() {
        let mut take = 0_usize;
        for (count, (at, character)) in rest.char_indices().enumerate() {
            take = at.saturating_add(character.len_utf8());
            if count.saturating_add(1) == width {
                break;
            }
        }
        let (front, back) = rest.split_at(take);
        let (next, _reasoning, _content) = decode_reasoning_delta(&state, Some(front), false);
        state = next;
        rest = back;
        width = if width == 3 {
            1_usize
        } else {
            width.saturating_add(1)
        };
    }
    let (_drained, _reasoning, _content) = decode_reasoning_delta(&state, None, true);
});
