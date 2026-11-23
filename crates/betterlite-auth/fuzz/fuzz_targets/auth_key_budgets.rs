#![no_main]

use betterlite_auth::auth_key_budgets::expiry::parse_temp_budget_expiry;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return;
    };
    drop(parse_temp_budget_expiry(text));
});
