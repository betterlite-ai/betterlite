#![cfg(test)]

use rstest::rstest;
use serde_json::Value;

use super::{OK, run};

#[rstest]
#[case::text_and_content_array(
    r#"{"model":"gpt-5","messages":[
        {"role":"system","content":"be brief"},
        {"role":"developer","content":[{"type":"text","text":"line"},"bare",{"type":"text","text":5},{"type":"other"}]},
        {"role":"user","content":["bare",{"type":"text","text":"hi"},{"type":"text","text":5}]},
        {"role":"alien","content":"ignored"}]}"#,
    200, OK,
    Some(("Bearer sk-deployment", r#"{"model":"gpt-5","input":[{"role":"user","content":"bare\nhi\n"}],"instructions":"be brief\nline\nbare\n"}"#)),
    r#"tokens=7 original={"input_tokens":7}"#,
)]
#[case::image_and_file_parts(
    r#"{"model":"gpt-5","messages":[
        {"role":"user","content":[
            {"type":"image_url","image_url":{"url":"http://i/1.png","detail":"high"}},
            {"type":"image_url","image_url":{"url":"http://i/2.png","detail":""}},
            {"type":"image_url","image_url":"http://i/4.png"},
            {"type":"image_url","image_url":{"url":""}},
            {"type":"file","file":{"filename":"a.txt","file_data":"QQ=="}},
            {"type":"file","file":{"filename":"a.txt"}},
            {"type":"video"},{"type":"text","text":"cap"}]},
        {"role":"assistant","content":[{"type":"image_url","image_url":"http://i/3.png"},{"type":"text","text":"seen"}]}]}"#,
    200, OK,
    Some(("Bearer sk-deployment", r#"{"model":"gpt-5","input":[
        {"role":"user","content":[
            {"type":"input_image","image_url":"http://i/1.png","detail":"high"},
            {"type":"input_image","image_url":"http://i/2.png","detail":"auto"},
            {"type":"input_image","image_url":"http://i/4.png","detail":"auto"},
            {"type":"input_file","filename":"a.txt","file_data":"QQ=="},
            {"type":"input_text","text":"cap"}]},
        {"role":"assistant","content":"seen"}]}"#)),
    r#"tokens=7 original={"input_tokens":7}"#,
)]
#[case::tool_definitions_and_results(
    r#"{"model":"gpt-5","tools":[
        {"type":"function","function":{"name":"f","strict":true}},
        {"type":"function","function":{"name":"g","description":"d","parameters":{"type":"object"}}},
        {"type":"web_search"}],
      "messages":[{"role":"user","content":"hi"},
        {"role":"assistant","content":"calling","tool_calls":[{"id":"c1","function":{"name":"f","arguments":"{}"}},{}]},
        {"role":"tool","tool_call_id":"c1","content":"done"}]}"#,
    200, OK,
    Some(("Bearer sk-deployment", r#"{"model":"gpt-5","tools":[
        {"type":"function","name":"f","description":"","parameters":{},"strict":true},
        {"type":"function","name":"g","description":"d","parameters":{"type":"object"}},
        {"type":"web_search"}],
      "input":[{"role":"user","content":"hi"},{"role":"assistant","content":"calling"},
        {"type":"function_call","call_id":"c1","name":"f","arguments":"{}"},
        {"type":"function_call","call_id":"","name":"","arguments":""},
        {"type":"function_call_output","call_id":"c1","output":"done"}]}"#)),
    r#"tokens=7 original={"input_tokens":7}"#,
)]
#[case::missing_model(
    r#"{"model":"","messages":[{"role":"user","content":"hi"}]}"#,
    200,
    OK,
    None,
    "error 500: CountTokens processing error: model parameter is required"
)]
#[case::native_api_selected(
    r#"{"model":"gpt-5","provider":"openai","messages":[{"role":"user","content":"hi"}]}"#,
    200, OK,
    Some(("Bearer sk-deployment", r#"{"model":"gpt-5","input":[{"role":"user","content":"hi"}]}"#)),
    r#"native=true tokens=7 original={"input_tokens":7}"#,
)]
#[case::native_count_success(
    r#"{"model":"gpt-5","messages":[{"role":"user","content":"hi"}]}"#,
    200, r#"{"object":"response.input_tokens"}"#,
    Some(("Bearer sk-deployment", r#"{"model":"gpt-5","input":[{"role":"user","content":"hi"}]}"#)),
    r#"tokens=0 original={"object":"response.input_tokens"}"#,
)]
#[case::count_failure_zero_status(
    r#"{"model":"gpt-5","messages":[{"role":"user","content":"hi"}]}"#,
    429, "slow down",
    Some(("Bearer sk-deployment", r#"{"model":"gpt-5","input":[{"role":"user","content":"hi"}]}"#)),
    "error 429: slow down",
)]
#[case::missing_input_returns_none(
    r#"{"model":"gpt-5","system":"fallback","messages":[{"role":"system","content":"only"}]}"#,
    200,
    OK,
    None,
    "none"
)]
#[case::credentials_ignore_module_overrides(
    r#"{"model":"gpt-5","key":"","env_key":"sk-env","messages":[{"role":"user","content":"hi"}]}"#,
    200, OK,
    Some(("Bearer sk-env", r#"{"model":"gpt-5","input":[{"role":"user","content":"hi"}]}"#)),
    r#"tokens=7 original={"input_tokens":7}"#,
)]
#[case::python_string_tool_output(
    r#"{"model":"gpt-5","messages":[
        {"role":"tool","tool_call_id":"a","content":[{"type":"text","text":"it's"}]},
        {"role":"tool","tool_call_id":"b","content":[{"z":null,"a":true,"n":1.5,"l":[1,"x"]}]},
        {"role":"tool","tool_call_id":"c","content":[]}]}"#,
    200, OK,
    Some(("Bearer sk-deployment", r#"{"model":"gpt-5","input":[
        {"type":"function_call_output","call_id":"a","output":"[{'type': 'text', 'text': \"it's\"}]"},
        {"type":"function_call_output","call_id":"b","output":"[{'z': None, 'a': True, 'n': 1.5, 'l': [1, 'x']}]"},
        {"type":"function_call_output","call_id":"c","output":""}]}"#)),
    r#"tokens=7 original={"input_tokens":7}"#,
)]
#[case::count_http_headers_timeout(
    r#"{"model":"gpt-5","key":"sk-other","messages":[{"role":"user","content":"hi"}]}"#,
    200, OK,
    Some(("Bearer sk-other", r#"{"model":"gpt-5","input":[{"role":"user","content":"hi"}]}"#)),
    r#"tokens=7 original={"input_tokens":7}"#,
)]
#[case::request_optional_presence(
    r#"{"model":"gpt-5","system":"ignored","tools":[],"messages":[
        {"role":"system","content":""},{"role":"user","content":"hi"},{"role":"assistant","content":""}]}"#,
    200, OK,
    Some(("Bearer sk-deployment", r#"{"model":"gpt-5","tools":[],"instructions":"","input":[{"role":"user","content":"hi"},{"role":"assistant","content":""}]}"#)),
    r#"tokens=7 original={"input_tokens":7}"#,
)]
#[tokio::test]
async fn count_tokens_cases(
    #[case] scenario: &str,
    #[case] status: u16,
    #[case] answer: &str,
    #[case] sent: Option<(&str, &str)>,
    #[case] result: &str,
) {
    let (rendered, request) = run(scenario, status, answer).await;
    assert_eq!(rendered, result);
    let expected = sent.map(|(authorization, body)| {
        (
            authorization.to_owned(),
            serde_json::from_str::<Value>(body).unwrap(),
        )
    });
    assert_eq!(request, expected);
}
