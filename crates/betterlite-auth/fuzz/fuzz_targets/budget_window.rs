#![no_main]

use betterlite_auth::budget_primitives::window::decode_budget_window;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_budget_window(bytes));
});
