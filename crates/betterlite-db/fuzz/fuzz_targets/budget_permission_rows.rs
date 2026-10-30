#![no_main]

use betterlite_db::budget_permission_rows::decode_budget_permission_rows;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _ = decode_budget_permission_rows(bytes);
});
