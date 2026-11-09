#![no_main]

use betterlite_tokenizer::media::wire::TokenMedia;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _media = TokenMedia::try_from(bytes);
});
