#![no_main]

use betterlite_db::queues::pod_lock::owner::decode_pod_owner;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _owner = decode_pod_owner(bytes);
});
