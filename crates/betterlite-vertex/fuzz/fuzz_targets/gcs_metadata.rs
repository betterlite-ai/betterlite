#![no_main]

use betterlite_vertex::gemini_media::gcs_metadata::read_content_type;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _explicit = read_content_type(bytes, true);
    let _anonymous = read_content_type(bytes, false);
});
