#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use http::header::{CONTENT_LENGTH, HeaderMap, HeaderValue};
use rstest::rstest;
use uuid::Uuid;

use crate::vertex_batch_download::records::{ConversionContext, transform_file_content_response};
use crate::vertex_batch_download::streaming::{
    DownloadDecoder, DownloadMode, decode_download_chunk, finish_download,
};

const URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-2.5-flash%2Fbatch.jsonl";
const NOW: i64 = 1_700_000_000;

fn context() -> ConversionContext {
    let mut issued = 0_u128;
    ConversionContext {
        now: NOW,
        ids: Box::new(move || {
            issued = issued.saturating_add(1);
            Uuid::from_u128(issued)
        }),
    }
}

fn lines(bytes: &[u8]) -> usize {
    bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.trim_ascii().is_empty())
        .count()
}

#[rstest]
fn replays_the_vertex_batch_download_corpus(
    #[files("fuzz/corpus/vertex_batch_download/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    let Some((chunk_size, rest)) = bytes.split_first() else {
        return;
    };
    let Some((peek_limit, content)) = rest.split_first() else {
        return;
    };
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_LENGTH, HeaderValue::from(1_usize));
    let mut decoder =
        DownloadDecoder::new(headers.clone(), None, usize::from(*peek_limit), context());
    let mut output = Vec::new();
    for chunk in content.chunks(usize::from(*chunk_size).max(1)) {
        let produced = decode_download_chunk(&mut decoder, chunk);
        output.extend_from_slice(&produced);
    }
    let produced = finish_download(&mut decoder);
    output.extend_from_slice(&produced);
    match decoder.mode {
        DownloadMode::Passthrough => assert_eq!(output, content),
        DownloadMode::Lines => assert_eq!(lines(&output), lines(content)),
        DownloadMode::Embeddings | DownloadMode::Undecided => {}
    }
    let _buffered = transform_file_content_response(content, URL, &headers, false, &mut context());
}
