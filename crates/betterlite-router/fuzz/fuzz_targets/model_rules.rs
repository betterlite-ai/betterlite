#![no_main]

use betterlite_router::generalization_integration::model_generalizations::ModelGeneralizations;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _outcome = ModelGeneralizations::install(data);
});
