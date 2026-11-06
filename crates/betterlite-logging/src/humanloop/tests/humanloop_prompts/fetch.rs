use std::error::Error as _;
use std::io;

use betterlite_models::prompt_contracts::PromptManagementClient;
use rstest::rstest;
use secrecy::SecretString;

use super::support::{
    FakeTransport, PROMPT_BODY, Recorded, fake_clock, json, manager, params, replying,
};
use crate::humanloop::prompt_manager::FetchError;

fn fetch_prompt(status: u16, body: &str) -> Result<PromptManagementClient, FetchError<io::Error>> {
    let clock = fake_clock();
    let (transport, _recorded) = replying(status, body);
    let mut prompts = manager(transport, &clock, 60);
    let prompt = prompts.get_prompt_from_id("pr_abc123", &SecretString::from("hl-key"))?;
    Ok(prompt.clone())
}

#[test]
fn cache_miss_sends_a_get_with_the_x_api_key_header() {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, 60);
    prompts
        .get_prompt_from_id("pr_abc123", &SecretString::from("hl-key"))
        .expect("prompt");
    let fetches = recorded.lock().expect("lock");
    assert_eq!(fetches.len(), 1);
    assert_eq!(
        fetches[0].url,
        "https://api.humanloop.com/v5/prompts/pr_abc123"
    );
    assert_eq!(
        fetches[0].headers.get("X-Api-Key").map(String::as_str),
        Some("hl-key")
    );
    assert_eq!(
        fetches[0].headers.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
}

#[rstest]
#[case::slash("a/b", "a%2Fb")]
#[case::query("a?x=1", "a%3Fx%3D1")]
#[case::dots("..", "%2E%2E")]
#[case::unreserved("pr_a-b~c", "pr_a-b~c")]
fn prompt_id_is_a_single_encoded_path_segment(#[case] prompt_id: &str, #[case] encoded: &str) {
    let clock = fake_clock();
    let (transport, recorded) = replying(200, PROMPT_BODY);
    let mut prompts = manager(transport, &clock, 60);
    prompts
        .get_prompt_from_id(prompt_id, &SecretString::from("hl-key"))
        .expect("prompt");
    let url = recorded.lock().expect("lock")[0].url.clone();
    let expected = format!("https://api.humanloop.com/v5/prompts/{encoded}");
    assert_eq!(url, expected);
}

#[rstest]
#[case::bad_request(400, "bad id")]
#[case::unauthorized(401, "{\"detail\": \"no key\"}")]
#[case::not_found(404, "")]
#[case::server_error(500, "boom")]
#[case::redirect(302, "moved")]
#[case::informational(199, "early")]
fn a_non_2xx_response_raises_the_humanloop_error_with_the_response_text(
    #[case] status: u16,
    #[case] body: &str,
) {
    let error = fetch_prompt(status, body).expect_err("status");
    assert!(matches!(error, FetchError::Api { .. }));
    assert_eq!(
        error.to_string(),
        format!("Error getting prompt from Humanloop: {body}")
    );
}

#[rstest]
#[case::ok(200)]
#[case::created(201)]
#[case::last_success(299)]
fn a_2xx_response_is_accepted(#[case] status: u16) {
    fetch_prompt(status, PROMPT_BODY).expect("2xx");
}

#[test]
fn a_dict_template_becomes_a_single_item_list() {
    let body = r#"{"model": "gpt-4o", "template": {"role": "system", "content": "hi"}}"#;
    let prompt = fetch_prompt(200, body).expect("prompt");
    assert_eq!(
        prompt.prompt_template,
        vec![json(r#"{"role": "system", "content": "hi"}"#)]
    );
}

#[test]
fn a_list_template_is_used_as_is() {
    let prompt = fetch_prompt(200, PROMPT_BODY).expect("prompt");
    assert_eq!(
        prompt.prompt_template,
        vec![
            json(r#"{"role": "system", "content": "You help {{name}}."}"#),
            json(r#"{"role": "user", "content": "Say hello"}"#),
        ]
    );
}

#[rstest]
#[case::string(r#""text""#, "<class 'str'>")]
#[case::integer("3", "<class 'int'>")]
#[case::float("1.5", "<class 'float'>")]
#[case::null("null", "<class 'NoneType'>")]
#[case::boolean("true", "<class 'bool'>")]
fn an_invalid_template_type_raises_a_value_error_with_the_type(
    #[case] template: &str,
    #[case] type_name: &str,
) {
    let body = format!(r#"{{"model": "gpt-4o", "template": {template}}}"#);
    let error = fetch_prompt(200, &body).expect_err("template");
    assert!(matches!(error, FetchError::InvalidTemplateType { .. }));
    assert_eq!(
        error.to_string(),
        format!("Invalid template message type: {type_name}")
    );
}

#[test]
fn the_model_comes_from_the_response() {
    let prompt = fetch_prompt(200, PROMPT_BODY).expect("prompt");
    assert_eq!(prompt.prompt_template_model.as_deref(), Some("gpt-4o"));
    assert_eq!(prompt.prompt_id.as_deref(), Some("pr_abc123"));
}

#[test]
fn a_null_model_is_kept_as_none() {
    let body = r#"{"model": null, "template": []}"#;
    let prompt = fetch_prompt(200, body).expect("prompt");
    assert_eq!(prompt.prompt_template_model, None);
}

#[rstest]
#[case::no_template(
    r#"{"model": "gpt-4o"}"#,
    "the Humanloop response has no template field"
)]
#[case::no_model(r#"{"template": []}"#, "the Humanloop response has no model field")]
#[case::numeric_model(
    r#"{"model": 4, "template": []}"#,
    "the Humanloop response field model must be a string or null"
)]
fn a_malformed_prompt_response_names_the_field(#[case] body: &str, #[case] message: &str) {
    let error = fetch_prompt(200, body).expect_err("response");
    assert_eq!(error.to_string(), message);
}

#[rstest]
#[case::not_json("not json")]
#[case::list("[]")]
fn a_body_that_is_not_a_json_object_is_a_decode_error(#[case] body: &str) {
    let error = fetch_prompt(200, body).expect_err("body");
    assert!(matches!(error, FetchError::Decode { .. }));
}

#[test]
fn optional_params_are_the_response_keys_openai_chat_completion_accepts() {
    let prompt = fetch_prompt(200, PROMPT_BODY).expect("prompt");
    assert_eq!(
        prompt.prompt_optional_params,
        Some(params(r#"{"temperature": 0.2, "max_tokens": 64}"#))
    );
}

#[test]
fn a_transport_failure_keeps_its_source() {
    let clock = fake_clock();
    let transport = FakeTransport {
        status: None,
        body: String::new(),
        recorded: Recorded::default(),
    };
    let mut prompts = manager(transport, &clock, 60);
    let error = prompts
        .get_prompt_from_id("pr_abc123", &SecretString::from("hl-key"))
        .expect_err("transport");
    assert!(matches!(error, FetchError::Transport { .. }));
    assert!(error.source().is_some());
}
