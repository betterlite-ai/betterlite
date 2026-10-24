#![no_main]
use betterlite_server::auth::parse_key_header;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    for name in [
        "authorization",
        "api-key",
        "x-api-key",
        "x-goog-api-key",
        "ocp-apim-subscription-key",
        "x-litellm-api-key",
    ] {
        let _key = parse_key_header(name, bytes);
    }
});
