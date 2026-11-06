#![no_main]

use betterlite_http::hub_oauth::stored_tokens::parse::parse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(file) = parse(bytes) {
        for section in file.with_defaults() {
            for (index, (name, _value)) in section.fields.iter().enumerate() {
                assert!(
                    section
                        .fields
                        .iter()
                        .skip(index.saturating_add(1))
                        .all(|(other, _)| other != name)
                );
            }
        }
    }
});
