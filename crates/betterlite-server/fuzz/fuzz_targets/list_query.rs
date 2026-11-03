#![no_main]

use betterlite_server::list_api::plan::query::decode_list_query;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|bytes: &[u8]| {
    let decoded = decode_list_query(bytes);
    for (key, value) in &decoded.last_value {
        let last_pair = decoded
            .pairs
            .iter()
            .rev()
            .find(|(pair_key, _)| pair_key == key);
        assert_eq!(last_pair.map(|(_, pair_value)| pair_value), Some(value));
    }
});
