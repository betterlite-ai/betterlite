use std::fs;

use crate::gdc_auth::key::decode_gdc_key;
use crate::gdc_auth::token::decode_gdc_token_reply;

#[test]
fn replays_gdc_auth_payload_corpus() {
    let seen = fs::read_dir("fuzz/corpus/gdc_auth_payload")
        .expect("the gdc auth payload corpus")
        .map(|entry| entry.expect("a corpus entry").path())
        .filter(|path| path.is_file())
        .map(|path| fs::read(path).expect("a corpus file"))
        .map(|bytes| {
            let Some((&tag, payload)) = bytes.split_first() else {
                return;
            };
            if tag.is_multiple_of(2) {
                let _key = decode_gdc_key(payload, "gdc/fuzz");
            } else {
                let _reply = decode_gdc_token_reply(payload);
            }
        })
        .count();
    assert!(seen > 0_usize);
}
