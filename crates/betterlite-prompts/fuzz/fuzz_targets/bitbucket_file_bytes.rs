#![no_main]

use betterlite_prompts::bitbucket_client::content::decode_body;
use betterlite_prompts::bitbucket_client::path::sanitize_file_path;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if bytes.len() > 0x0001_0000 {
        return;
    }
    drop(decode_body(None, bytes));
    drop(decode_body(Some(b"text/plain"), bytes));
    if let Ok(file_path) = str::from_utf8(bytes) {
        drop(sanitize_file_path(file_path));
    }
});
