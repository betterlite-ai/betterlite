#![no_main]

use libfuzzer_sys::fuzz_target;
use std::str;

#[path = "../blog_feed_xml_support/date_parse.rs"]
mod date_parse;
#[path = "../blog_feed_xml_support/parser.rs"]
mod parser;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(xml) = str::from_utf8(bytes) {
        let _result = parser::parse_rss_to_posts(xml, 1);
    }
});
