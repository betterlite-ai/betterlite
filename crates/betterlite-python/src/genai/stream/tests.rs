#![cfg(test)]

use betterlite_core::google_genai::streaming::GoogleGenAiStream;
use betterlite_http::error::HttpError;
use pyo3::prelude::*;

use super::{NativeGenaiStream, StreamKind};

fn stream_of(events: Vec<&[u8]>) -> NativeGenaiStream {
    let source: Vec<Result<Vec<u8>, HttpError>> =
        events.into_iter().map(<[u8]>::to_vec).map(Ok).collect();
    NativeGenaiStream::new(StreamKind::Blocking(GoogleGenAiStream::new(
        source.into_iter(),
        1024,
    )))
}

#[test]
fn a_chunk_is_the_bytes_of_one_event() {
    Python::initialize();
    Python::attach(|py| {
        let stream = stream_of(vec![b"data: {}\n\n"]);
        let chunk = stream.next_chunk(py).unwrap().expect("one event");
        assert_eq!(chunk.as_bytes(py), &b"data: {}\n\n"[..]);
        assert!(stream.next_chunk(py).unwrap().is_none());
    });
}

#[test]
fn closing_stops_the_stream_before_its_end() {
    Python::initialize();
    Python::attach(|py| {
        let stream = stream_of(vec![b"data: {}\n\n"]);
        stream.close(py);
        stream.close(py);
        assert!(stream.next_chunk(py).unwrap().is_none());
    });
}
