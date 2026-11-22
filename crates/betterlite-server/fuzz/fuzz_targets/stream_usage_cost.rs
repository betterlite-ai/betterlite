#![no_main]

use betterlite_server::stream_usage_cost::testing::{FixedCalculator, FixedRecorder};
use betterlite_server::stream_usage_cost::{StreamCostInjector, decode_cost_frame};
use libfuzzer_sys::fuzz_target;
use rust_decimal::Decimal;

fuzz_target!(|data: &[u8]| {
    let recorder = FixedRecorder(Some(Decimal::ZERO));
    let calculator = FixedCalculator(Some(Decimal::ZERO));
    let injector = StreamCostInjector {
        enabled: true,
        recorder: &recorder,
        calculator: &calculator,
    };
    let _decoded = decode_cost_frame(injector, data, "gpt-4o", None);
    let disabled = StreamCostInjector {
        enabled: false,
        recorder: &recorder,
        calculator: &calculator,
    };
    let _untouched = decode_cost_frame(disabled, data, "gpt-4o", None);
});
