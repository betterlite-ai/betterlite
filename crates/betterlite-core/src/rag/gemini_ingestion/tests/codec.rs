use std::fs;
use std::path::PathBuf;

use crate::rag::gemini_ingestion::reply::{decode_upload_reply, upload_name};
use crate::rag::gemini_ingestion::wire;

#[test]
fn finalize_decodes_the_reply_in_the_reply_module() {
    assert_eq!(upload_name(br#"{"name":"files/1"}"#).unwrap(), "files/1");
    assert_eq!(wire::body_text(b"boom"), "boom");
    assert_eq!(wire::body_text(&[0xff]), "1 bytes that are not UTF-8");
    assert_eq!(
        decode_upload_reply(br#"{"name":"files/1"}"#).file_id,
        "files/1"
    );
}

#[rstest::rstest]
fn replays_the_gemini_upload_reply_corpus(
    #[files("fuzz/corpus/gemini_upload_reply/*")] path: PathBuf,
) {
    let bytes = fs::read(path).unwrap();
    drop(decode_upload_reply(&bytes));
}
