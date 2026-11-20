#![no_main]

use betterlite_server::key_policy_fields::columns::decode_json_column_bytes;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_json_column_bytes("router_settings", bytes));
    drop(decode_json_column_bytes("budget_limits", bytes));
    drop(decode_json_column_bytes("metadata", bytes));
});
