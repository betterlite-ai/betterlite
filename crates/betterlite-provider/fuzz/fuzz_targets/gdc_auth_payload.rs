#![no_main]

use betterlite_provider::gdc_auth::key::decode_gdc_key;
use betterlite_provider::gdc_auth::token::decode_gdc_token_reply;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Some((&tag, payload)) = bytes.split_first() else {
        return;
    };
    if tag.is_multiple_of(2) {
        let _key = decode_gdc_key(payload, "gdc/fuzz");
    } else {
        let _reply = decode_gdc_token_reply(payload);
    }
});
