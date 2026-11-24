#![no_main]

use std::str::from_utf8;

use betterlite_server::spend::sessions::cursor::decode_session_cursor;
use betterlite_server::spend::sessions::query::decode_session_query;
use libfuzzer_sys::fuzz_target;

// the first line names the parser: a raw query string or a stored session cursor follows it
fuzz_target!(|bytes: &[u8]| {
    let mut parts = bytes.splitn(2, |byte| *byte == b'\n');
    let tag = parts.next().unwrap_or_default();
    let rest = parts.next().unwrap_or_default();
    match tag {
        b"query" => {
            let Ok(text) = from_utf8(rest) else {
                return;
            };
            drop(decode_session_query(Some(text)));
        }
        b"cursor" => {
            let Ok(text) = from_utf8(rest) else {
                return;
            };
            drop(decode_session_cursor(Some(text)));
        }
        _ => {}
    }
});
