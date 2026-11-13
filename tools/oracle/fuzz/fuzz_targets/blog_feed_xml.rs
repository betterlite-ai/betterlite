#![no_main]

use libfuzzer_sys::fuzz_target;
use std::str;

#[path = "../../src/helpers/server/blog_feed/date_parse.rs"]
#[expect(
    clippy::absolute_paths,
    reason = "The oracle's lib.rs expects the same lint for this included helper file."
)]
mod date_parse;
#[path = "../../src/helpers/server/blog_feed/parser.rs"]
#[expect(
    clippy::disallowed_methods,
    clippy::absolute_paths,
    reason = "The oracle's lib.rs expects the same lints for this included helper file."
)]
mod parser;

fuzz_target!(|bytes: &[u8]| {
    if let Ok(xml) = str::from_utf8(bytes) {
        let _result = parser::parse_rss_to_posts(xml, 1);
    }
});
