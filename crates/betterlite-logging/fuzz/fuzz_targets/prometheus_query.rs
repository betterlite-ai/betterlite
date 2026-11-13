#![no_main]

use betterlite_logging::prometheus_query::response_records::PrometheusQueryResponse;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(serde_json::from_slice::<PrometheusQueryResponse>(bytes));
});
