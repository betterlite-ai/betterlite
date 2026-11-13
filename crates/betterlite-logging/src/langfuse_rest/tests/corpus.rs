use std::fs;
use std::path::PathBuf;

use rstest::rstest;

use crate::langfuse_rest::records::{Projects, Prompt};
use crate::langfuse_rest::transport::HttpResponse;

#[rstest]
fn replays_the_langfuse_response_corpus(#[files("fuzz/corpus/langfuse_response/*")] path: PathBuf) {
    let body = fs::read(&path).expect("the corpus file");
    let response = HttpResponse {
        status: 200,
        body: body.clone(),
    };
    let _decoded = response.json();
    let _prompt = serde_json::from_slice::<Prompt>(&body);
    let _projects = serde_json::from_slice::<Projects>(&body);
}
