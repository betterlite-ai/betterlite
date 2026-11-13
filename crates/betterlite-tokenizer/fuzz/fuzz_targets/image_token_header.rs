#![no_main]

use betterlite_tokenizer::image::dimensions::image_dimensions;
use betterlite_tokenizer::image::header::image_type;
use betterlite_tokenizer::image::settings::Settings;
use libfuzzer_sys::fuzz_target;
use std::hint::black_box;

fuzz_target!(|bytes: &[u8]| {
    black_box(image_type(bytes));
    drop(image_dimensions(bytes, &Settings::default()));
});
