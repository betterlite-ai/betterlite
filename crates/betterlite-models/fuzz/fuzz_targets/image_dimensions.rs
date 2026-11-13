#![no_main]

use betterlite_models::image_cost::default_calculator::parse_image_dimensions;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|value: &str| {
    let _outcome = parse_image_dimensions(value);
});
