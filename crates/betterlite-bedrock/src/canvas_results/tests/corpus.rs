#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::canvas_results::decode_canvas_reply;

#[rstest]
fn replays_the_bedrock_other_canvas_results_corpus(
    #[files("fuzz/corpus/bedrock_other_canvas_results/*")] path: PathBuf,
) {
    let name = path.file_name().and_then(|name| name.to_str()).unwrap();
    let bytes = fs::read(&path).unwrap();
    let accepted = name.starts_with("valid") || name.starts_with("boundary");
    assert_eq!(decode_canvas_reply(&bytes).is_ok(), accepted, "{name}");
    if name == "valid-images.json" {
        let reply = decode_canvas_reply(&bytes).unwrap();
        let Json::Object(fields) = reply else {
            panic!("valid-images.json holds an object");
        };
        assert_eq!(
            fields.get("images"),
            Some(&Json::List(vec![
                Json::Text("aGVsbG8=".to_owned()),
                Json::Text("d29ybGQ=".to_owned()),
            ]))
        );
        assert!(fields.get("errors").is_none());
    }
}
