#![no_main]

use betterlite_openai::text_completion_stream::{ChatTextStream, decode_chat_events};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _whole = decode_chat_events(bytes, "model", true);
    let _unused = decode_chat_events(bytes, "", false);
    if let Some((first, rest)) = bytes.split_first() {
        let boundary = usize::from(*first) % (rest.len() + 1);
        let (left, right) = rest.split_at(boundary);
        let mut stream = ChatTextStream::new(String::from("model"), true);
        if let Ok(mut chunks) = stream.push(left) {
            if let Ok(more) = stream.push(right) {
                chunks.extend(more);
            }
            let _finished = stream.finish();
        }
    }
});
