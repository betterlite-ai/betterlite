#![cfg(test)]

use std::fs;
use std::io::Error;
use std::path::PathBuf;

use betterlite_http::proxy::limits::TransportLimits;

use super::{MAX_EVAL_REPLY_BYTES, decode_eval_reply};
use crate::evals::error::EvalsError;

#[test]
fn decode_eval_reply_refuses_a_body_over_the_transport_limit() {
    assert_eq!(MAX_EVAL_REPLY_BYTES, TransportLimits::DEFAULT.body_bytes);
    let at_limit = vec![b' '; MAX_EVAL_REPLY_BYTES];
    assert!(matches!(
        decode_eval_reply(&at_limit),
        Err(EvalsError::ReplyJson { .. })
    ));
    let over_limit = vec![b' '; MAX_EVAL_REPLY_BYTES.saturating_add(1)];
    assert!(matches!(
        decode_eval_reply(&over_limit),
        Err(EvalsError::ReplyTooLarge { size, limit })
            if size == MAX_EVAL_REPLY_BYTES.saturating_add(1) && limit == MAX_EVAL_REPLY_BYTES
    ));
}

#[test]
fn decode_eval_reply_accepts_eval_and_run_json() {
    let eval = decode_eval_reply(br#"{"id": "eval-1", "object": "eval", "created_at": 1}"#);
    assert_eq!(eval.unwrap()["id"], "eval-1");
    let run = decode_eval_reply(br#"{"id": "run-1", "object": "eval.run", "status": "queued"}"#);
    assert_eq!(run.unwrap()["status"], "queued");
}

#[test]
fn replays_the_openai_evals_corpus() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/openai_evals");
    let mut paths = fs::read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "no eval reply corpus seeds");
    paths
        .into_iter()
        .try_for_each(|path| {
            let bytes = fs::read(&path)?;
            let name = path.file_name().unwrap().to_str().unwrap();
            let decoded = decode_eval_reply(&bytes);
            if name.starts_with("valid-") {
                assert!(decoded.is_ok(), "{name}: {decoded:?}");
            } else {
                assert!(
                    matches!(decoded, Err(EvalsError::ReplyJson { .. })),
                    "{name}: {decoded:?}"
                );
            }
            Ok::<(), Error>(())
        })
        .unwrap();
}
