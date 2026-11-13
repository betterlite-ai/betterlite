#![no_main]

use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_upload_validation::batch::decode_file_upload_validation;
use betterlite_core::api::file_upload_validation::line_shape::LineShape;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_file_upload_validation(
        bytes,
        LineShape::OpenAi,
        ApiLimits::default(),
    ));
    drop(decode_file_upload_validation(
        bytes,
        LineShape::VertexPassthrough,
        ApiLimits::default(),
    ));
});
