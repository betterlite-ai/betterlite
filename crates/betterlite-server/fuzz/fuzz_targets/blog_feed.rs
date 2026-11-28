#![no_main]

use betterlite_server::public_endpoints::blog_feed_parser::parse_rss_to_posts;
use libfuzzer_sys::fuzz_target;
use std::str;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(xml) = str::from_utf8(bytes) {
        let _result = parse_rss_to_posts(xml, 1, 64);
    }
});
