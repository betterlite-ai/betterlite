#![no_main]

use betterlite_server::config_fields::http::decode_body;
use betterlite_server::config_fields::records::{ConfigFieldDelete, ConfigFieldUpdate};
use betterlite_server::config_fields::validation::validate_field;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|body: &[u8]| {
    if let Ok(update) = decode_body::<ConfigFieldUpdate>(body) {
        drop(validate_field(&update.field_name, &update.field_value));
    }
    drop(decode_body::<ConfigFieldDelete>(body));
});
