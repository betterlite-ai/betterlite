#![no_main]

use betterlite_core::rag::gemini_ingestion::reply::decode_upload_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_upload_reply(bytes));
});
