use std::fs;
use std::path::PathBuf;
use std::str::from_utf8;

use betterlite_core::api::api_boundaries::ApiLimits;
use rstest::rstest;

use crate::passthrough::request_data::parse_request_by_content_type;

#[rstest]
fn replays_the_passthrough_request_data_corpus(
    #[files("fuzz/corpus/passthrough_request_data/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("corpus seed is readable");
    let Some((length_bytes, rest)) = bytes.split_first_chunk::<2>() else {
        panic!("{} has no content type length", path.display());
    };
    let header_len = usize::from(u16::from_be_bytes(*length_bytes));
    let Some((header, body)) = rest.split_at_checked(header_len) else {
        panic!(
            "{} names a content type longer than the seed",
            path.display()
        );
    };
    let content_type = from_utf8(header).expect("the seed's content type is text");
    drop(parse_request_by_content_type(
        content_type,
        body,
        &[],
        &ApiLimits::default(),
    ));
}
