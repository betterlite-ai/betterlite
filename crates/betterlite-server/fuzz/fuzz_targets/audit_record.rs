#![no_main]
use betterlite_server::audit_events::record::decode_audit_record;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _record = decode_audit_record(bytes);
});
