#![no_main]
use betterlite_bedrock::web_identity::claims::decode_web_identity_claims;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let Ok(token) = std::str::from_utf8(bytes) else {
        return;
    };
    let _claims = decode_web_identity_claims(token);
});
