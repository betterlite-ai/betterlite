use std::fs;
use std::path::PathBuf;

use rstest::rstest;
use tokio::runtime::Builder;

use crate::count::settings::Settings;
use crate::count::wire::TokenRequestWire;

use super::support;

#[rstest]
fn replays_the_token_request_corpus(#[files("fuzz/corpus/token_request/*")] path: PathBuf) {
    let bytes = fs::read(path).unwrap();
    let Ok(wire) = TokenRequestWire::try_from(bytes.as_slice()) else {
        return;
    };
    let directory = tempfile::tempdir().unwrap();
    let counter = support::counter(Settings::default(), directory.path());
    let Ok(selected) = wire.selected() else {
        return;
    };
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    let _counted = runtime.block_on(counter.count(&wire.request(selected.as_ref())));
}
