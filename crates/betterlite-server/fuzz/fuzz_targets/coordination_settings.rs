#![no_main]

use betterlite_server::coordination_redis_settings::decode::{
    decode_coordination_settings, decode_persisted_coordination_settings,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_coordination_settings(bytes));
    drop(decode_persisted_coordination_settings(bytes));
});
