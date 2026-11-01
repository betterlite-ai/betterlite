#![no_main]

use libfuzzer_sys::fuzz_target;

#[path = "../../src/helpers/server/blog_feed/date_parse.rs"]
mod date_parse;
#[path = "../../src/helpers/server/blog_feed/parser.rs"]
mod parser;

fuzz_target!(|bytes: &[u8]| {
    let xml = String::from_utf8_lossy(bytes);
    let _result = parser::parse_rss_to_posts(&xml, 1);
});
