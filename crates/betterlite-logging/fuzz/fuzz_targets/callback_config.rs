#![no_main]

use betterlite_logging::callback_config::FeatureFlags;
use betterlite_logging::callback_config::parse::{
    validate_logging_metadata, validate_team_callback,
};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let flags = FeatureFlags {
        otel_v2_enabled: true,
    };
    let _metadata = validate_logging_metadata(bytes, flags);
    let _team = validate_team_callback(bytes, flags);
});
