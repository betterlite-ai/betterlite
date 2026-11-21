#![cfg(test)]

use pyo3::prelude::*;

use super::NativeChatStream;

#[test]
fn the_blocking_stream_gives_its_chunks_then_none() {
    Python::initialize();
    Python::attach(|py| {
        let stream = NativeChatStream::new(vec![String::from("first"), String::from("second")]);
        assert_eq!(stream.next_chunk(py), Some(String::from("first")));
        assert_eq!(stream.next_chunk(py), Some(String::from("second")));
        assert_eq!(stream.next_chunk(py), None);
    });
}

#[test]
fn closing_the_stream_ends_it_before_its_chunks_run_out() {
    Python::initialize();
    Python::attach(|py| {
        let stream = NativeChatStream::new(vec![String::from("first")]);
        stream.close(py);
        assert_eq!(stream.next_chunk(py), None);
    });
}
