use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::health_access_and_connections::connection_test::ConnectionTestRequest;
use crate::workflow_runs::router::decode_body;

#[rstest]
fn replays_the_health_connection_body_corpus(
    #[files("fuzz/corpus/health_connection_body/*")] path: PathBuf,
) {
    let bytes = fs::read(path).expect("the corpus seed reads");
    drop(decode_body::<ConnectionTestRequest>(&bytes));
}
