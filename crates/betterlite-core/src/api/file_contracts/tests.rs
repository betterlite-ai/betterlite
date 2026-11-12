#![cfg(test)]

use std::io::{Error, Read, Result};

use rstest::rstest;

use super::{is_batch_jsonl_file, is_batch_jsonl_request};

struct LazyFile;

impl Read for LazyFile {
    fn read(&mut self, _buffer: &mut [u8]) -> Result<usize> {
        Err(Error::other("the batch predicate read the file"))
    }
}

#[derive(Debug, Clone, Copy)]
enum Probe {
    EmptyBytes,
    Name,
    Missing,
    Lazy,
}

fn metadata_result(purpose: Option<&str>, content_type: Option<&str>, probe: Probe) -> bool {
    match probe {
        Probe::EmptyBytes => is_batch_jsonl_request(purpose, content_type, Some(&Vec::<u8>::new())),
        Probe::Name => {
            is_batch_jsonl_request(purpose, content_type, Some(&String::from("upload.bin")))
        }
        Probe::Missing => is_batch_jsonl_request::<LazyFile>(purpose, content_type, None),
        Probe::Lazy => is_batch_jsonl_request(purpose, content_type, Some(&LazyFile)),
    }
}

#[rstest]
#[case::jsonl(Some("batch"), Some("application/jsonl"), Some(b"".as_slice()), true)]
#[case::json(Some("batch"), Some("application/json"), Some(b"".as_slice()), true)]
#[case::octet_stream(Some("batch"), Some("application/octet-stream"), Some(b"".as_slice()), true)]
#[case::ndjson(Some("batch"), Some("application/x-ndjson"), Some(b"".as_slice()), true)]
#[case::jsonlines(Some("batch"), Some("application/x-jsonlines"), Some(b"".as_slice()), true)]
#[case::plain_text(Some("batch"), Some("text/plain"), Some(b"".as_slice()), true)]
#[case::jsonl_without_content(Some("batch"), Some("application/jsonl"), None, false)]
#[case::json_without_content(Some("batch"), Some("application/json"), None, false)]
#[case::octet_stream_without_content(Some("batch"), Some("application/octet-stream"), None, false)]
#[case::ndjson_without_content(Some("batch"), Some("application/x-ndjson"), None, false)]
#[case::jsonlines_without_content(Some("batch"), Some("application/x-jsonlines"), None, false)]
#[case::plain_text_without_content(Some("batch"), Some("text/plain"), None, false)]
#[case::assistants_purpose(Some("assistants"), Some("text/plain"), Some(b"".as_slice()), false)]
#[case::capitalized_purpose(Some("Batch"), Some("text/plain"), Some(b"".as_slice()), false)]
#[case::null_purpose(None, Some("text/plain"), Some(b"".as_slice()), false)]
#[case::absent_purpose(None, Some("text/plain"), Some(b"".as_slice()), false)]
#[case::null_content_type(Some("batch"), None, Some(b"".as_slice()), false)]
#[case::csv_content_type(Some("batch"), Some("text/csv"), Some(b"".as_slice()), false)]
#[case::capitalized_content_type(Some("batch"), Some("TEXT/PLAIN"), Some(b"".as_slice()), false)]
#[case::parameterized_content_type(
    Some("batch"),
    Some("application/json;charset=utf-8"),
    Some(b"".as_slice()),
    false
)]
fn batch_extracted_predicate(
    #[case] purpose: Option<&str>,
    #[case] content_type: Option<&str>,
    #[case] content: Option<&[u8]>,
    #[case] expected: bool,
) {
    assert_eq!(
        is_batch_jsonl_file(purpose, content_type, content),
        expected
    );
}

#[rstest]
#[case::empty_bytes(Some("batch"), Some("text/plain"), Probe::EmptyBytes, true)]
#[case::file_name(Some("batch"), Some("text/plain"), Probe::Name, true)]
#[case::null_file(Some("batch"), Some("text/plain"), Probe::Missing, false)]
#[case::absent_file(Some("batch"), Some("text/plain"), Probe::Missing, false)]
#[case::lazy_file_is_not_read(Some("batch"), Some("text/plain"), Probe::Lazy, true)]
#[case::assistants_purpose(Some("assistants"), Some("text/plain"), Probe::Name, false)]
#[case::capitalized_purpose(Some("Batch"), Some("text/plain"), Probe::Name, false)]
#[case::null_purpose(None, Some("text/plain"), Probe::Name, false)]
#[case::absent_purpose(None, Some("text/plain"), Probe::Name, false)]
#[case::null_content_type(Some("batch"), None, Probe::Name, false)]
#[case::csv_content_type(Some("batch"), Some("text/csv"), Probe::Name, false)]
#[case::capitalized_content_type(Some("batch"), Some("TEXT/PLAIN"), Probe::Name, false)]
#[case::parameterized_content_type(
    Some("batch"),
    Some("application/json;charset=utf-8"),
    Probe::Name,
    false
)]
fn batch_metadata_predicate(
    #[case] purpose: Option<&str>,
    #[case] content_type: Option<&str>,
    #[case] probe: Probe,
    #[case] expected: bool,
) {
    assert_eq!(metadata_result(purpose, content_type, probe), expected);
}
