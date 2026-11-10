#![no_main]

use std::collections::VecDeque;

use betterlite_openai::aleph_alpha::lines::LineDecoder;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Some((&first, body)) = bytes.split_first() else {
        return;
    };
    let cut = usize::from(first).min(body.len());
    let (head, tail) = body.split_at(cut);
    let mut whole = LineDecoder::new(usize::MAX);
    let mut whole_lines = VecDeque::new();
    let _whole_pushed = whole.push(body, &mut whole_lines);
    whole.flush(&mut whole_lines);
    let mut split = LineDecoder::new(usize::MAX);
    let mut split_lines = VecDeque::new();
    let _head_pushed = split.push(head, &mut split_lines);
    let _tail_pushed = split.push(tail, &mut split_lines);
    split.flush(&mut split_lines);
    assert_eq!(whole_lines, split_lines);
});
