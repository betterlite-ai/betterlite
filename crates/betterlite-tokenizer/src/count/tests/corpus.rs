use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::count::settings::Settings;
use crate::count::wire::TokenRequestWire;

use super::support;

#[rstest]
#[tokio::test]
async fn replays_the_token_request_corpus(#[files("fuzz/corpus/token_request/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(wire) = TokenRequestWire::try_from(bytes.as_slice()) else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let counter = support::counter(Settings::default(), directory.path());
    let Ok(selected) = wire.selected() else {
        return;
    };
    let _counted = counter.count(&wire.request(selected.as_ref())).await;
}
