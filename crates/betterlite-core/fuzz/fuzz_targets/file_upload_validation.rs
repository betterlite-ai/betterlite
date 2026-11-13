#![no_main]
use betterlite_core::api::api_boundaries::ApiLimits;
use betterlite_core::api::file_upload_validation::batch::decode_file_upload_validation;
use betterlite_core::api::file_upload_validation::line_shape::LineShape;
use libfuzzer_sys::fuzz_target;
fuzz_target!(|bytes: &[u8]| {
    for shape in [LineShape::OpenAi, LineShape::VertexPassthrough] {
        drop(decode_file_upload_validation(
            bytes,
            shape,
            ApiLimits::default(),
        ));
    }
});
