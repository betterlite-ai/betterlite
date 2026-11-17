#![no_main]

use betterlite_openai::output_limits::{ErrorBody, decode_rejected_fields};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _rejected = decode_rejected_fields(ErrorBody::Http(bytes));
});
