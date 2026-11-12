#![no_main]

use betterlite_bedrock::image_call::decode::decode_bedrock_image_reply;
use betterlite_bedrock::image_models::family::Family;
use libfuzzer_sys::fuzz_target;

const FAMILIES: [Family; 4] = [
    Family::Titan,
    Family::Stability1,
    Family::Stability3,
    Family::Nova,
];

fuzz_target!(|bytes: &[u8]| {
    for family in FAMILIES {
        let _outcome = decode_bedrock_image_reply(bytes, family);
    }
});
