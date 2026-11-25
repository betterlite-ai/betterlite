#![no_main]

use betterlite_server::lens_admin_routes::{body, preview, workers};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(body::decode_content_query(bytes));
    drop(body::decode::<preview::Preview>(bytes));
    drop(body::decode::<workers::WorkerBilling>(bytes));
    drop(body::decode::<workers::WorkerName>(bytes));
});
