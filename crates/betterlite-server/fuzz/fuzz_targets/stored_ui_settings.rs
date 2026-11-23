#![no_main]

use betterlite_server::ui_settings_state::decode::decode_stored_ui_settings;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    drop(decode_stored_ui_settings(data));
});
