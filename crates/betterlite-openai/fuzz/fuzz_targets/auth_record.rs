#![no_main]

use betterlite_openai::chatgpt::token_store::codec::{decode_auth_record, decode_token_claims};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _record = decode_auth_record(bytes);
    let _claims = decode_token_claims(bytes);
});
