#![no_main]

use betterlite_db::spend_partitions::partition_bound::decode_partition_bound;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let _bound = decode_partition_bound(bytes);
});
