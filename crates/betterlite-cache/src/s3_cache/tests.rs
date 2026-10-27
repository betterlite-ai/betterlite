#![cfg(test)]

use super::decode_body;

#[test]
fn replays_the_s3_cache_body_corpus() {
    let corpus = include_bytes!("../../fuzz/corpus/s3_cache_body/json_object");
    drop(decode_body(corpus));
}
