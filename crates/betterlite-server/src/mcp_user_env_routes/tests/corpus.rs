use std::ffi::OsStr;
use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::mcp_user_env_routes::body::decode_user_env_vars_request;

#[rstest]
fn replays_the_mcp_user_env_vars_body_corpus(
    #[files("fuzz/corpus/mcp_user_env_vars_body/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).expect("a corpus seed is readable");
    let accepted = decode_user_env_vars_request(None, &bytes).is_ok();
    let Some(name) = path.file_name().and_then(OsStr::to_str) else {
        return;
    };
    if name.starts_with("accepted-") {
        assert!(accepted, "{name}");
    }
    if name.starts_with("refused-") {
        assert!(!accepted, "{name}");
    }
}
