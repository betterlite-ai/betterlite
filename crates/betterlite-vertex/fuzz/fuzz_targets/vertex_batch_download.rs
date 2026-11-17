#![no_main]

use betterlite_vertex::vertex_batch_download::records::ConversionContext;
use betterlite_vertex::vertex_batch_download::records::transform_file_content_response; // ast-grep-ignore: no-long-names
use betterlite_vertex::vertex_batch_download::streaming::decode_download_chunk;
use betterlite_vertex::vertex_batch_download::streaming::{DownloadDecoder, finish_download};
use http::header::{CONTENT_LENGTH, HeaderMap, HeaderValue};
use libfuzzer_sys::fuzz_target;
use uuid::Uuid;

const URL: &str = "https://storage.googleapis.com/storage/v1/b/b/o/litellm-vertex-files%2Fpublishers%2Fgoogle%2Fmodels%2Fgemini-2.5-flash%2Fbatch.jsonl";

fn context() -> ConversionContext {
    let mut issued = 0_u128;
    ConversionContext {
        now: 1_700_000_000,
        ids: Box::new(move || {
            issued = issued.saturating_add(1);
            Uuid::from_u128(issued)
        }),
    }
}

fuzz_target!(|bytes: &[u8]| {
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
    // ast-grep-ignore: no-long-names
    drop(transform_file_content_response(
        content,
        URL,
        &headers,
        false,
        &mut context(),
    ));
    drop(output);
});
