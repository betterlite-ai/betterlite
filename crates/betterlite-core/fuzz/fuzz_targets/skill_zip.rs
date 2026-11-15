#![no_main]

use betterlite_core::skills::content::decode::decode_skill_zip;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let Ok(mut archive) = decode_skill_zip(bytes.to_vec()) else {
        return;
    };
    let Ok(names) = archive.entry_names() else {
        return;
    };
    for (index, name) in names.iter().enumerate() {
        let _read = archive.read_bytes(index, name);
    }
});
