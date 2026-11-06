#![cfg(test)]

use std::fs;
use std::io;
use std::path::Path;

use crate::threads::decode::decode_thread_message_reply;
use crate::threads::error::{ReplyFault, ThreadsError};
use crate::threads::records::ThreadMessage;

// The file extension names the outcome a seed must produce.
#[test]
fn replays_the_openai_threads_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_threads");
    let mut replayed = 0_usize;
    fs::read_dir(corpus)
        .unwrap()
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let outcome = match decode_thread_message_reply::<ThreadMessage>(&bytes) {
                Ok(_message) => "ok",
                Err(ThreadsError::Reply { fault, .. }) => match fault {
                    ReplyFault::Malformed => "malformed",
                    ReplyFault::Truncated => "truncated",
                    ReplyFault::Shape => "shape",
                },
                Err(other) => panic!("{} failed outside decoding: {other}", path.display()),
            };
            let name = path.file_name().unwrap().to_str().unwrap();
            assert_eq!(
                path.extension().unwrap().to_str().unwrap(),
                outcome,
                "{name}"
            );
            replayed += 1;
            Ok::<(), io::Error>(())
        })
        .unwrap();
    assert!(replayed >= 8, "{replayed} seeds replayed");
}
