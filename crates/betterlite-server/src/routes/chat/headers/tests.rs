#![cfg(test)]

use std::collections::BTreeMap;
use std::time::Duration;

use axum::http::HeaderMap;
use betterlite_config::loader::ConfigLoader;
use betterlite_openai::chat::deployment::{ChatDeployment, OpenAiDefaults};
use rstest::rstest;

use super::stream_headers;

#[rstest]
#[case::overhead(1500, "0.266")]
#[case::saturates(1000, "0.000")]
fn stream_timing_headers(#[case] total_micros: u64, #[case] overhead: &str) {
    let config = ConfigLoader {
        lookup: BTreeMap::<String, String>::new(),
    }
    .load("model_list:\n  - model_name: g\n    litellm_params:\n      model: openai/up\n")
    .unwrap();
    let deployment = ChatDeployment::resolve(
        config.model_list.first().unwrap(),
        &OpenAiDefaults::default(),
    )
    .unwrap();
    let headers = stream_headers(
        &HeaderMap::new(),
        &deployment,
        "stream-timing",
        Duration::from_micros(1234),
        Duration::from_micros(total_micros),
    );
    assert_eq!(
        headers.get("x-litellm-response-duration-ms").unwrap(),
        "1.234"
    );
    assert_eq!(
        headers.get("x-litellm-overhead-duration-ms").unwrap(),
        overhead
    );
}
