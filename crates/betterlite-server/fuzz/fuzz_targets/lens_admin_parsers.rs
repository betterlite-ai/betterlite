#![no_main]

use betterlite_server::lens_admin_routes::body::{decode, decode_content_query};
use betterlite_server::lens_admin_routes::preview::Preview;
use betterlite_server::lens_admin_routes::workers::{WorkerBilling, WorkerName};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    drop(decode_content_query(bytes));
    drop(decode::<Preview>(bytes));
    drop(decode::<WorkerBilling>(bytes));
    drop(decode::<WorkerName>(bytes));
});
