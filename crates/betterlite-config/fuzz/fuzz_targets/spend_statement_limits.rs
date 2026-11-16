#![no_main]
use betterlite_config::spend::decode_spend_statement_limits;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    let Ok(limits) = decode_spend_statement_limits(bytes) else {
        return;
    };
    assert!(limits.max_bytes >= 1);
    assert!(limits.max_rows >= 1);
});
