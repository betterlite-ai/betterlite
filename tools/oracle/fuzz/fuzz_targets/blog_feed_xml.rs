#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../blog_feed_xml_support/utf8.rs"]
mod utf8;

#[path = "../blog_feed_xml_support/date_parse.rs"]
mod date_parse;
#[path = "../blog_feed_xml_support/parser.rs"]
mod parser;

fuzz_target!(|bytes: &[u8]| {
    let xml = utf8::lossy_utf8(bytes);
    let _result = parser::parse_rss_to_posts(&xml, 1);
});
