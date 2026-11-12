use std::collections::BTreeMap;

use rstest::rstest;

use super::support::{key, messages, object};
use crate::langflow_chat::call::{PreparedRun, RunInput, prepare_run};
use crate::langflow_chat::config::STREAM_PARAM_IN_BODY;
use crate::langflow_chat::error::LangFlowError;

fn run(api_base: Option<&str>, options: &str, stream: Option<bool>) -> PreparedRun {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional_params = object(options);
    prepare_run(&RunInput {
        model: "langflow/abc",
        messages: &messages,
        optional_params: &optional_params,
        api_base,
        api_key: Some(&key("secret")),
        stream,
    })
    .expect("the fixture run prepares")
}

#[test]
fn prepare_run_assembles_the_langflow_call() {
    let prepared = run(
        Some("http://host:7860"),
        r#"{"input_type":"text"}"#,
        Some(true),
    );
    assert_eq!(prepared.url, "http://host:7860/api/v1/run/abc");
    assert_eq!(
        prepared.headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(
        prepared.headers.get("x-api-key").map(String::as_str),
        Some("secret")
    );
    assert_eq!(prepared.body.input_value, "hi");
    assert!(prepared.fake_stream);
}

#[rstest]
#[case::explicit_stream(Some(true), true)]
#[case::explicit_no_stream(Some(false), false)]
#[case::absent_stream(None, false)]
fn streaming_is_faked_only_for_an_explicit_stream_request(
    #[case] stream: Option<bool>,
    #[case] fake: bool,
) {
    assert_eq!(
        run(Some("http://host:7860"), "{}", stream).fake_stream,
        fake
    );
}

#[test]
fn the_stream_flag_never_goes_into_the_body() {
    assert!(!STREAM_PARAM_IN_BODY);
}

#[test]
fn prepare_run_requires_a_base_before_it_reads_the_options() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional_params = object(r#"{"tweaks":{"x":{}}}"#);
    let error = prepare_run(&RunInput {
        model: "langflow/abc",
        messages: &messages,
        optional_params: &optional_params,
        api_base: None,
        api_key: None,
        stream: None,
    })
    .expect_err("without a base there is no URL");
    assert!(matches!(error, LangFlowError::ApiBaseRequired));
}

#[test]
fn prepare_run_refuses_tweaks() {
    let messages = messages(r#"[{"role":"user","content":"hi"}]"#);
    let optional_params = object(r#"{"tweaks":{"x":{}}}"#);
    let error = prepare_run(&RunInput {
        model: "langflow/abc",
        messages: &messages,
        optional_params: &optional_params,
        api_base: Some("http://host:7860"),
        api_key: None,
        stream: None,
    })
    .expect_err("a request parameter cannot override the flow components");
    assert!(matches!(
        error,
        LangFlowError::Api {
            status_code: 400,
            ..
        }
    ));
}
