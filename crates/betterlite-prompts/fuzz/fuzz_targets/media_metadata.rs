#![no_main]

use betterlite_prompts::media_metadata::url_mime::infer_content_type;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let split = bytes.len().min(32);
    let (url_bytes, content) = bytes.split_at(split);
    let url = str::from_utf8(url_bytes).unwrap_or_default();
    drop(infer_content_type(url, content, None));
});
