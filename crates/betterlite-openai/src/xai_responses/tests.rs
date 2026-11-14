#![cfg(test)]

use std::future::Future;

use betterlite_config::sections::xai_identity::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::error::XaiResponsesError;
use super::{
    OauthAccess, get_complete_url, map_openai_params, restate_event_usage, restate_reply_usage,
    supports_native_websocket, validate_environment,
};
use crate::api::responses_event_records::records::KnownResponseEvent;
use crate::api::responses_objects::ResponsesApiResponse;
use crate::openai_base_config::{Headers, Object};
use crate::response_events::ReplyBody;
use crate::response_events::ResponsesReply;
use crate::response_events::event_decode::{DecodedStreamResponse, decode_response_event};
use crate::xai_tokens::error::XaiOauthError;

fn settings(
    xai: Option<&str>,
    generic: Option<&str>,
    environment: Option<&str>,
    base: Option<&str>,
) -> Settings {
    Settings {
        xai_key: xai.map(SecretString::from),
        generic_api_key: generic.map(SecretString::from),
        environment_key: environment.map(SecretString::from),
        api_base: base.map(str::to_owned),
    }
}

fn tools(params: &Object) -> &Vec<Value> {
    match params.get("tools") {
        Some(Value::Array(items)) => items,
        None
        | Some(
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) | Value::Object(_),
        ) => unreachable!("the transformed params hold a tools list"),
    }
}

fn mapped(json: Value) -> Object {
    let mut params = json.as_object().cloned().unwrap_or_default();
    map_openai_params(&mut params);
    params
}

fn response(json: Value) -> ResponsesApiResponse {
    serde_json::from_value(json).unwrap()
}

fn base_response(usage: Value) -> Value {
    json!({
        "id": "resp-1",
        "created_at": 7,
        "output": [],
        "usage": usage,
    })
}

fn restated(json: Value) -> ResponsesApiResponse {
    let mut reply = ResponsesReply {
        response: ReplyBody::Typed(Box::new(response(json))),
        headers: IndexMap::new(),
        additional_headers: IndexMap::new(),
    };
    restate_reply_usage(&mut reply);
    let ReplyBody::Typed(restated) = reply.response else {
        unreachable!("a typed reply stays typed");
    };
    *restated
}

fn cost(response: &ResponsesApiResponse) -> Option<Decimal> {
    response.usage.as_ref().and_then(|usage| usage.cost)
}

fn ticks(json: Value) -> Json {
    serde_json::from_value(json).unwrap()
}

#[test]
fn metadata_is_removed_even_when_null() {
    let transformed = mapped(json!({"metadata": null, "store": false, "model": "grok-4"}));
    assert!(!transformed.contains_key("metadata"));
    assert_eq!(transformed.get("store"), Some(&Value::Bool(false)));
    assert_eq!(
        transformed.get("model"),
        Some(&Value::String(String::from("grok-4")))
    );
}

#[test]
fn code_interpreter_keeps_only_its_type() {
    let transformed = mapped(json!({
        "tools": [{"type": "code_interpreter", "container": {"type": "auto"}, "extra": 1}],
    }));
    assert_eq!(
        tools(&transformed),
        &vec![json!({"type": "code_interpreter"})]
    );
}

#[test]
fn nested_filters_win_over_flat_fields() {
    let transformed = mapped(json!({
        "tools": [{
            "type": "web_search",
            "filters": {"allowed_domains": ["nested"], "excluded_domains": ["gone"]},
            "allowed_domains": ["flat"],
            "excluded_domains": ["flat-gone"],
            "search_context_size": "low",
            "enable_image_understanding": true,
            "unknown": 1,
        }],
    }));
    assert_eq!(
        tools(&transformed),
        &vec![json!({
            "type": "web_search",
            "filters": {"allowed_domains": ["nested"], "excluded_domains": ["gone"]},
            "enable_image_understanding": true,
        })]
    );
}

#[test]
fn flat_domains_fill_filters_without_a_mapping() {
    let transformed = mapped(json!({
        "tools": [{
            "type": "web_search",
            "filters": null,
            "allowed_domains": ["flat"],
            "excluded_domains": ["flat-gone"],
        }],
    }));
    assert_eq!(
        tools(&transformed),
        &vec![json!({
            "type": "web_search",
            "filters": {"allowed_domains": ["flat"], "excluded_domains": ["flat-gone"]},
        })]
    );
}

#[test]
fn an_empty_filters_mapping_suppresses_flat_domains() {
    let transformed = mapped(json!({
        "tools": [{"type": "web_search", "filters": {}, "allowed_domains": ["flat"]}],
    }));
    assert_eq!(tools(&transformed), &vec![json!({"type": "web_search"})]);
}

#[test]
fn a_filters_mapping_keeps_an_explicit_null_domain_list() {
    let transformed = mapped(json!({
        "tools": [{"type": "web_search", "filters": {"allowed_domains": null}}],
    }));
    assert_eq!(
        tools(&transformed),
        &vec![json!({"type": "web_search", "filters": {"allowed_domains": null}})]
    );
}

#[test]
fn x_search_keeps_every_handle_date_and_media_field() {
    let transformed = mapped(json!({
        "tools": [{
            "type": "x_search",
            "allowed_x_handles": ["a"],
            "excluded_x_handles": null,
            "from_date": "2026-01-01",
            "to_date": "2026-02-01",
            "enable_image_understanding": false,
            "enable_video_understanding": null,
            "unknown": "dropped",
        }],
    }));
    assert_eq!(
        tools(&transformed),
        &vec![json!({
            "type": "x_search",
            "allowed_x_handles": ["a"],
            "excluded_x_handles": null,
            "from_date": "2026-01-01",
            "to_date": "2026-02-01",
            "enable_image_understanding": false,
            "enable_video_understanding": null,
        })]
    );
}

#[test]
fn unknown_tools_and_non_objects_pass_through() {
    let function = json!({"type": "function", "name": "f", "parameters": {}});
    let transformed = mapped(json!({"tools": [function.clone(), "plain", 3]}));
    assert_eq!(
        tools(&transformed),
        &vec![function, json!("plain"), json!(3)]
    );
}

#[test]
fn a_truthy_non_list_tools_value_becomes_one_element() {
    let transformed = mapped(json!({
        "tools": {"type": "web_search", "allowed_domains": ["a"]},
    }));
    assert_eq!(
        tools(&transformed),
        &vec![json!({"type": "web_search", "filters": {"allowed_domains": ["a"]}})]
    );
}

#[rstest]
#[case::zero(json!(0))]
#[case::empty_text(json!(""))]
#[case::empty_object(json!({}))]
#[case::empty_list(json!([]))]
#[case::false_value(json!(false))]
#[case::null_value(Value::Null)]
fn falsy_tools_are_not_normalized(#[case] given: Value) {
    let transformed = mapped(json!({"tools": given.clone()}));
    assert_eq!(transformed.get("tools"), Some(&given));
}

#[test]
fn absent_tools_and_store_stay_absent() {
    let transformed = mapped(json!({"model": "grok-4"}));
    assert!(!transformed.contains_key("tools"));
    assert!(!transformed.contains_key("store"));
    let stored = mapped(json!({"store": true}));
    assert_eq!(stored.get("store"), Some(&Value::Bool(true)));
}

struct FixedOauth {
    fail: bool,
    calls: usize,
}

impl OauthAccess for FixedOauth {
    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send {
        self.calls += 1;
        let outcome = if self.fail {
            Err(XaiOauthError::LoginRequired)
        } else {
            Ok(SecretString::from("oauth-token"))
        };
        std::future::ready(outcome)
    }
}

fn oauth(fail: bool) -> FixedOauth {
    FixedOauth { fail, calls: 0 }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn key(text: &str) -> SecretString {
    SecretString::from(text)
}

#[tokio::test]
async fn the_explicit_key_replaces_an_existing_authorization() {
    let empty = settings(None, None, None, None);
    let mut oauth = oauth(false);
    let validated = validate_environment(
        headers(&[("Authorization", "Bearer old"), ("X-Keep", "yes")]),
        Some(&key("explicit")),
        "grok-4",
        false,
        &empty,
        &mut oauth,
    )
    .await
    .unwrap();
    assert_eq!(validated.get("Authorization").unwrap(), "Bearer explicit");
    assert_eq!(validated.get("X-Keep").unwrap(), "yes");
    assert_eq!(oauth.calls, 0);
}

#[tokio::test]
async fn the_generic_key_falls_back_before_the_environment() {
    let both = settings(None, Some("generic"), Some("environment"), None);
    let mut oauth = oauth(false);
    let validated = validate_environment(Headers::new(), None, "grok-4", false, &both, &mut oauth)
        .await
        .unwrap();
    assert_eq!(validated.get("Authorization").unwrap(), "Bearer generic");
    let environment_only = settings(None, None, Some("environment"), None);
    let mut oauth = oauth(false);
    let validated = validate_environment(
        Headers::new(),
        None,
        "grok-4",
        false,
        &environment_only,
        &mut oauth,
    )
    .await
    .unwrap();
    assert_eq!(
        validated.get("Authorization").unwrap(),
        "Bearer environment"
    );
}

#[tokio::test]
async fn a_module_key_beats_the_generic_and_environment_keys() {
    let configured = settings(Some("module"), Some("generic"), Some("environment"), None);
    let mut oauth = oauth(false);
    let validated = validate_environment(
        Headers::new(),
        None,
        "grok-4",
        false,
        &configured,
        &mut oauth,
    )
    .await
    .unwrap();
    assert_eq!(validated.get("Authorization").unwrap(), "Bearer module");
}

#[tokio::test]
async fn a_missing_key_raises_without_a_status() {
    let empty = settings(None, None, None, None);
    let mut oauth = oauth(false);
    let error = validate_environment(Headers::new(), None, "grok-4", false, &empty, &mut oauth)
        .await
        .unwrap_err();
    assert!(matches!(error, XaiResponsesError::MissingApiKey));
    assert_eq!(error.status_code(), None);
    assert_eq!(oauth.calls, 0);
}

#[tokio::test]
async fn an_opted_in_login_supplies_the_key_only_without_one() {
    let empty = settings(None, None, None, None);
    let mut oauth = oauth(false);
    let validated = validate_environment(Headers::new(), None, "grok-4", true, &empty, &mut oauth)
        .await
        .unwrap();
    assert_eq!(
        validated.get("Authorization").unwrap(),
        "Bearer oauth-token"
    );
    assert_eq!(oauth.calls, 1);
    let mut oauth = oauth(false);
    let validated = validate_environment(
        Headers::new(),
        Some(&key("explicit")),
        "grok-4",
        true,
        &empty,
        &mut oauth,
    )
    .await
    .unwrap();
    assert_eq!(validated.get("Authorization").unwrap(), "Bearer explicit");
    assert_eq!(oauth.calls, 0);
}

#[tokio::test]
async fn an_oauth_failure_is_an_authentication_error() {
    let empty = settings(None, None, None, None);
    let mut oauth = oauth(true);
    let error = validate_environment(Headers::new(), None, "grok-4", true, &empty, &mut oauth)
        .await
        .unwrap_err();
    assert!(matches!(error, XaiResponsesError::Authentication { .. }));
    assert_eq!(error.status_code(), Some(401));
    assert!(!format!("{error:?}").contains("oauth-token"));
}

#[rstest]
#[case::explicit(
    Some("https://explicit.example/v1"),
    Some("https://global.example/v1"),
    None,
    "https://explicit.example/v1/responses"
)]
#[case::global(
    None,
    Some("https://global.example/v1"),
    None,
    "https://global.example/v1/responses"
)]
#[case::environment(
    None,
    None,
    Some("https://env.example/v1"),
    "https://env.example/v1/responses"
)]
#[case::built_in(None, None, None, "https://api.x.ai/v1/responses")]
#[case::trailing_slashes(
    None,
    Some("https://global.example/v1///"),
    None,
    "https://global.example/v1/responses"
)]
fn the_normal_base_order_prefers_the_closest_setting(
    #[case] api_base: Option<&str>,
    #[case] litellm_api_base: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let configured = settings(None, None, None, environment);
    assert_eq!(
        get_complete_url(
            api_base,
            litellm_api_base,
            None,
            false,
            &configured,
            "https://oauth.example/v1",
        ),
        expected
    );
}

#[test]
fn oauth_selects_its_own_base_only_without_a_key() {
    let configured = settings(None, Some("generic"), Some("https://env.example/v1"), None);
    assert_eq!(
        get_complete_url(
            Some("https://explicit.example/v1"),
            None,
            None,
            true,
            &configured,
            "https://oauth.example/v1/",
        ),
        "https://oauth.example/v1/responses"
    );
    assert_eq!(
        get_complete_url(
            Some("https://explicit.example/v1"),
            None,
            None,
            false,
            &configured,
            "https://oauth.example/v1",
        ),
        "https://explicit.example/v1/responses"
    );
}

#[test]
fn an_ordinary_reply_restates_the_reported_ticks() {
    let replied = restated(base_response(json!({
        "input_tokens": 1,
        "output_tokens": 2,
        "total_tokens": 3,
        "cost_in_usd_ticks": 12_500_000_000_i64,
    })));
    assert_eq!(cost(&replied), Some(Decimal::new(125, 2)));
}

#[rstest]
#[case::negative(json!(-1))]
#[case::boolean(json!(true))]
#[case::fraction(json!(1.5))]
#[case::text(json!("100"))]
#[case::absent(Value::Null)]
fn a_reply_without_valid_ticks_keeps_its_cost(#[case] given: Value) {
    let mut usage = json!({
        "input_tokens": 1,
        "output_tokens": 2,
        "total_tokens": 3,
        "cost": 0.25,
    });
    if !given.is_null() {
        usage["cost_in_usd_ticks"] = given;
    }
    let replied = restated(base_response(usage));
    assert_eq!(cost(&replied), Some(Decimal::new(25, 2)));
}

#[test]
fn a_reply_carrying_no_usage_stays_without_one() {
    let mut body = base_response(json!({}));
    body.as_object_mut().unwrap().remove("usage");
    let replied = restated(body);
    assert_eq!(cost(&replied), None);
}

#[test]
fn a_lax_reply_is_left_as_sent() {
    let sent = ticks(json!({"cost_in_usd_ticks": 12_500_000_000_i64}));
    let mut reply = ResponsesReply {
        response: ReplyBody::Lax(IndexMap::from([(String::from("usage"), sent.clone())])),
        headers: IndexMap::new(),
        additional_headers: IndexMap::new(),
    };
    restate_reply_usage(&mut reply);
    let ReplyBody::Lax(fields) = reply.response else {
        unreachable!("a lax reply stays lax");
    };
    assert_eq!(fields.get("usage"), Some(&sent));
}

fn event(json: Value) -> DecodedStreamResponse {
    let bytes = serde_json::to_vec(&json).unwrap();
    decode_response_event(&bytes).unwrap()
}

fn terminal_event(event_type: &str) -> Value {
    json!({
        "type": event_type,
        "response": base_response(json!({
            "input_tokens": 1,
            "output_tokens": 2,
            "total_tokens": 3,
            "cost_in_usd_ticks": 5_000_000_000_i64,
        })),
    })
}

fn record_cost(decoded: &DecodedStreamResponse) -> Option<Decimal> {
    let DecodedStreamResponse::Known { record, .. } = decoded else {
        return None;
    };
    if let KnownResponseEvent::ResponseCompleted(fields)
    | KnownResponseEvent::ResponseIncomplete(fields)
    | KnownResponseEvent::ResponseFailed(fields) = record.as_ref()
    {
        return fields.response.usage.as_ref().and_then(|usage| usage.cost);
    }
    if let KnownResponseEvent::ResponseCreated(fields)
    | KnownResponseEvent::ResponseInProgress(fields) = record.as_ref()
    {
        return fields.response.usage.as_ref().and_then(|usage| usage.cost);
    }
    None
}

// a created or in-progress event carries ticks too, so only the terminal three may turn them into a cost
#[rstest]
#[case::completed("response.completed", Some(Decimal::new(5, 1)))]
#[case::incomplete("response.incomplete", Some(Decimal::new(5, 1)))]
#[case::failed("response.failed", Some(Decimal::new(5, 1)))]
#[case::created("response.created", None)]
#[case::in_progress("response.in_progress", None)]
fn only_terminal_events_restate_the_reported_ticks(
    #[case] event_type: &str,
    #[case] expected: Option<Decimal>,
) {
    let mut decoded = event(terminal_event(event_type));
    restate_event_usage(&mut decoded);
    assert_eq!(record_cost(&decoded), expected);
}

#[test]
fn a_terminal_event_without_usage_is_left_alone() {
    let mut decoded = event(json!({
        "type": "response.completed",
        "response": {"id": "resp-2", "created_at": 7, "output": []},
    }));
    restate_event_usage(&mut decoded);
    let DecodedStreamResponse::Known { record, .. } = decoded else {
        unreachable!("a valid terminal event decodes as known");
    };
    let KnownResponseEvent::ResponseCompleted(fields) = *record else {
        panic!("the completed event variant is the one that was decoded");
    };
    assert!(fields.response.usage.is_none());
}

#[test]
fn a_generic_event_is_left_alone() {
    let mut decoded = event(json!({"type": "response.shell_call.in_progress", "extra": 1}));
    restate_event_usage(&mut decoded);
    assert!(matches!(decoded, DecodedStreamResponse::Generic(_)));
}

#[test]
fn the_xai_responses_capability_is_not_native_websocket() {
    assert!(!supports_native_websocket());
}

#[test]
fn a_restated_cost_keeps_the_ticks_beside_it() {
    let replied = restated(base_response(json!({
        "input_tokens": 1,
        "output_tokens": 2,
        "total_tokens": 3,
        "cost_in_usd_ticks": 1_i64,
    })));
    let usage = replied.usage.unwrap();
    assert_eq!(usage.extra.get("cost_in_usd_ticks"), Some(&ticks(json!(1))));
    assert_eq!(usage.cost, Some(Decimal::new(1, 10)));
}
