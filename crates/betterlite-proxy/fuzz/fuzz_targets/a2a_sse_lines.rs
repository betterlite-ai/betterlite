#![no_main]

use betterlite_proxy::agent_endpoints::a2a_stream::lines::{LineSplitter, event_from_line};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let split = usize::from(bytes.first().copied().unwrap_or_default());
    let (head, tail) = bytes.split_at_checked(split).unwrap_or((bytes, &[]));
    let mut splitter = LineSplitter::default();
    let mut lines = splitter.push(head);
    lines.extend(splitter.push(tail));
    lines.extend(splitter.finish());
    for line in &lines {
        drop(event_from_line(line));
    }
});
