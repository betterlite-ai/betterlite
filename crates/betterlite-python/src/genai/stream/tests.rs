#![cfg(test)]

use betterlite_core::google_genai::streaming::GoogleGenAiStream;
use betterlite_http::error::HttpError;
use pyo3::prelude::*;

use super::{NativeGenaiStream, StreamKind};

fn stream_of(events: Vec<&'static [u8]>) -> NativeGenaiStream {
    let source = events
        .into_iter()
        .map(<[u8]>::to_vec)
        .map(Ok::<Vec<u8>, HttpError>);
    NativeGenaiStream::from(StreamKind::Blocking(Box::new(GoogleGenAiStream::new(
        source, 1024,
    ))))
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
fn two_events_arrive_one_per_call() {
    Python::initialize();
    Python::attach(|py| {
        let stream = stream_of(vec![b"data: 1\n\n", b"data: 2\n\n"]);
        let first = stream.next_chunk(py).unwrap().expect("the first event");
        assert_eq!(first.as_bytes(py), &b"data: 1\n\n"[..]);
        let second = stream.next_chunk(py).unwrap().expect("the second event");
        assert_eq!(second.as_bytes(py), &b"data: 2\n\n"[..]);
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
