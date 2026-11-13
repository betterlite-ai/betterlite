#![no_main]

use betterlite_openai::fine_tuning::job::decode_fine_tune_job;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _plain = decode_fine_tune_job(bytes, false);
    let _azure = decode_fine_tune_job(bytes, true);
});
