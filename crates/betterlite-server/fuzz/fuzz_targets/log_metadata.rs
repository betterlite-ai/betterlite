#![no_main]

use std::str::from_utf8;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_server::spend::log_payload::dates::decode_detail_dates;
use betterlite_server::spend::log_payload::metadata::{
    cold_storage_object_key, decode_log_metadata, hydrate_log_metadata,
};
use libfuzzer_sys::fuzz_target;

// stored metadata is a database column, so the fuzzer keeps to sizes a row can hold
const MAX_METADATA_BYTES: usize = 64 * 1024;

// the first line names the parser: metadata text follows it, or a start and an end date on their own lines
fuzz_target!(|bytes: &[u8]| {
    let mut parts = bytes.splitn(2, |byte| *byte == b'\n');
    let tag = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default();
    match tag {
        b"metadata" if rest.len() <= MAX_METADATA_BYTES => {
            let metadata = decode_log_metadata(rest);
            let Ok(encoded) = serde_json::to_vec(&metadata) else {
                return;
            };
            assert_eq!(decode_log_metadata(&encoded), metadata);
            if let Ok(text) = from_utf8(rest) {
                let mut stored = JsonData::String(text.to_owned());
                hydrate_log_metadata(&mut stored);
                assert_eq!(stored, metadata);
                drop(cold_storage_object_key(Some(&stored)));
            }
        }
        b"dates" => {
            let Ok(text) = from_utf8(rest) else {
                return;
            };
            let mut lines = text.splitn(2, '\n');
            let start = lines.next().filter(|line| !line.is_empty());
            let end = lines.next();
            drop(decode_detail_dates(start, end));
        }
        _ => {}
    }
});
