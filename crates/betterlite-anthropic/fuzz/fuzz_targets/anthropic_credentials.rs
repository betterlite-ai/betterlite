#![no_main]

use betterlite_anthropic::credentials::decode_anthropic_models;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &[u8]| {
    let Some((&selector, payload)) = input.split_first() else {
        return;
    };
    let status = 200_u16.saturating_add(u16::from(selector));
    let _outcome = decode_anthropic_models(status, payload);
});
