#![no_main]
use betterlite_core::api::file_batch_guardrails::lines::iter_records;
use betterlite_core::api::file_batch_guardrails::record_json::load_record;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    drop(load_record(bytes));
    for record in iter_records(bytes) {
        drop(record);
    }
});
