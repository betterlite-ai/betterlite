use betterlite_models::decisions::Json;
use rstest::rstest;

use crate::api::responses_provider_contract::config::ResponsesProviderConfig as _;
use crate::api::responses_provider_contract::records::{CreateRequest, Headers, IdRequest, Params};
use crate::manus_responses::error::ManusResponsesError;

use super::support::{params, plain_config};

fn json(wire: &str) -> Json {
    serde_json::from_str(wire).expect("the fixture is JSON")
}

fn create(model: &str, input: &str, optional: &str) -> CreateRequest {
    CreateRequest {
        model: model.to_owned(),
        input: json(input),
        optional_params: params(optional),
        litellm_params: Params::new(),
        headers: Headers::new(),
    }
}

fn body_of(model: &str, input: &str, optional: &str) -> Params {
    plain_config()
        .transform_create_request(&create(model, input, optional))
        .expect("the request transforms")
}

#[test]
fn the_create_body_gets_task_mode_and_the_agent_profile() {
    let body = body_of("manus/manus-1.6", r#""hi""#, r#"{"store": false}"#);
    assert_eq!(body.get("task_mode"), Some(&json(r#""agent""#)));
    assert_eq!(body.get("agent_profile"), Some(&json(r#""manus-1.6""#)));
    assert_eq!(body.get("model"), Some(&json(r#""manus/manus-1.6""#)));
    assert_eq!(body.get("input"), Some(&json(r#""hi""#)));
    assert_eq!(body.get("store"), Some(&json("false")));
}

#[test]
fn the_agent_profile_is_the_whole_model_without_a_slash() {
    let body = body_of("manus-1.6-lite", r#""hi""#, "{}");
    assert_eq!(
        body.get("agent_profile"),
        Some(&json(r#""manus-1.6-lite""#))
    );
}

#[test]
fn extra_body_keys_overwrite_task_mode_and_the_agent_profile() {
    let body = body_of(
        "manus/manus-1.6",
        r#""hi""#,
        r#"{"extra_body": {"task_mode": "chat", "agent_profile": "custom", "vendor": 7}}"#,
    );
    assert_eq!(body.get("task_mode"), Some(&json(r#""chat""#)));
    assert_eq!(body.get("agent_profile"), Some(&json(r#""custom""#)));
    assert_eq!(body.get("vendor"), Some(&json("7")));
}

#[test]
fn an_empty_extra_body_merges_nothing() {
    let body = body_of("manus/manus-1.6", r#""hi""#, r#"{"extra_body": {}}"#);
    assert_eq!(body.get("task_mode"), Some(&json(r#""agent""#)));
    assert_eq!(body.get("agent_profile"), Some(&json(r#""manus-1.6""#)));
}

#[test]
fn a_null_extra_body_merges_nothing() {
    let body = body_of("manus/manus-1.6", r#""hi""#, r#"{"extra_body": null}"#);
    assert_eq!(body.get("task_mode"), Some(&json(r#""agent""#)));
}

#[test]
fn a_non_object_extra_body_names_the_field() {
    let error = plain_config()
        .transform_create_request(&create(
            "manus/manus-1.6",
            r#""hi""#,
            r#"{"extra_body": 7}"#,
        ))
        .expect_err("a number merges no keys");
    assert_eq!(
        error.to_string(),
        "the extra_body parameter must be an object"
    );
    assert!(matches!(error, ManusResponsesError::ExtraBodyShape));
}

#[rstest]
#[case::number("7")]
#[case::object(r#"{"input": "hi"}"#)]
#[case::null("null")]
fn a_non_text_non_list_input_is_refused(#[case] input: &str) {
    let error = plain_config()
        .transform_create_request(&create("manus/manus-1.6", input, "{}"))
        .expect_err("the base builder only takes text or items");
    assert_eq!(
        error.to_string(),
        "the input parameter must be text or a list of items"
    );
    assert!(matches!(error, ManusResponsesError::InputShape));
}

#[test]
fn a_list_input_reaches_the_body_as_items() {
    let body = body_of(
        "manus/manus-1.6",
        r#"[{"role": "user", "content": "hi"}]"#,
        "{}",
    );
    assert_eq!(
        body.get("input"),
        Some(&json(r#"[{"role": "user", "content": "hi"}]"#))
    );
}

#[rstest]
#[case::plain("resp_1", "https://api.manus.im/v1/responses/resp_1")]
#[case::reserved("a/b c?d#e", "https://api.manus.im/v1/responses/a%2Fb%20c%3Fd%23e")]
#[case::unicode("r\u{e9}s", "https://api.manus.im/v1/responses/r%C3%A9s")]
fn the_get_url_appends_the_encoded_id(#[case] response_id: &str, #[case] expected: &str) {
    let request = IdRequest {
        response_id: response_id.to_owned(),
        api_base: String::from("https://api.manus.im/v1/responses"),
        litellm_params: Params::new(),
        headers: Headers::new(),
    };
    let prepared = plain_config()
        .transform_get_request(&request)
        .expect("the id encodes");
    assert_eq!(prepared.url, expected);
    assert!(prepared.params.is_empty());
}

#[test]
fn an_empty_response_id_names_the_field() {
    let request = IdRequest {
        response_id: String::new(),
        api_base: String::from("https://api.manus.im/v1/responses"),
        litellm_params: Params::new(),
        headers: Headers::new(),
    };
    let error = plain_config()
        .transform_get_request(&request)
        .expect_err("an empty id cannot be a path segment");
    assert!(matches!(
        error,
        ManusResponsesError::ResponseId { ref response_id, .. } if response_id.is_empty()
    ));
}
