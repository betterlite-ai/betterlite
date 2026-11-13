use std::fs;
use std::path::PathBuf;

use betterlite_logging::zerobus::client::token_response::decode_token_response;
use rstest::rstest;

#[rstest]
fn replays_the_zerobus_token_response_corpus(
    #[files("fuzz/corpus/zerobus_token_response/*")] path: PathBuf,
) {
    let bytes = fs::read(&path).unwrap();
    if let Err(failure) = decode_token_response(&bytes, 1_000.0) {
        assert!(
            !failure.retryable,
            "{} must decode or fail permanently",
            path.display()
        );
    }
}
