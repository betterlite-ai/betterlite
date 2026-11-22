use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::search_tool_routes::body::decode_body;
use crate::search_tool_routes::connection::ConnectionBody;
use crate::search_tool_routes::records::WriteBody;

#[rstest]
fn replays_the_search_tool_body_corpus(#[files("fuzz/corpus/search_tool_body/*")] path: PathBuf) {
    let raw = fs::read(&path).expect("a corpus file");
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return;
    };
    let accepted = if name.contains("connection") {
        decode_body::<ConnectionBody>(&raw).is_ok()
    } else {
        decode_body::<WriteBody>(&raw).is_ok()
    };
    assert_eq!(accepted, name.starts_with("accepted-"), "{name}");
}
