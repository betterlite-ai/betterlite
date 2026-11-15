#![cfg(test)]

use std::future::{Future, ready};

use betterlite_config::sections::xai_identity::Settings;
use betterlite_models::decisions::Json;
use indexmap::IndexMap;
use rstest::rstest;
use rust_decimal::Decimal;
use secrecy::SecretString;
use serde_json::{Value, json};

use super::error::XaiResponsesError;
use super::{
    Environment, OauthAccess, SUPPORTS_NATIVE_WEBSOCKET, UrlInput, get_complete_url,
    map_openai_params, restate_event_usage, restate_reply_usage, validate_environment,
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

// serde_json::json! uses Into::into for object members, which clippy.toml disallows
fn object<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Object(
        pairs
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

fn tools(params: &Object) -> &Vec<Value> {
    params
        .get("tools")
        .and_then(Value::as_array)
        .expect("the transformed params hold a tools list")
}

fn mapped(given: &Value) -> Object {
    let mut params = given.as_object().cloned().unwrap_or_default();
    map_openai_params(&mut params);
    params
}

fn response(body: Value) -> ResponsesApiResponse {
    serde_json::from_value(body).unwrap()
}

fn base_response(usage: &Value) -> Value {
    object([
        ("id", json!("resp-1")),
        ("created_at", json!(7_i32)),
        ("output", json!([])),
        ("usage", usage.clone()),
    ])
}

fn restated(body: Value) -> ResponsesApiResponse {
    let mut reply = ResponsesReply {
        response: ReplyBody::Typed(Box::new(response(body))),
        headers: IndexMap::new(),
        additional_headers: IndexMap::new(),
    };
    restate_reply_usage(&mut reply);
    let ReplyBody::Typed(restated) = reply.response else {
        panic!("a typed reply stays typed");
    };
    *restated
}

fn cost(response: &ResponsesApiResponse) -> Option<Decimal> {
    response.usage.as_ref()?.cost
}

fn ticks(body: Value) -> Json {
    serde_json::from_value(body).unwrap()
}

#[test]
fn metadata_is_removed_even_when_null() {
    let transformed = mapped(&object([
        ("metadata", Value::Null),
        ("store", json!(false)),
        ("model", json!("grok-4")),
    ]));
    assert!(!transformed.contains_key("metadata"));
    assert_eq!(transformed.get("store"), Some(&Value::Bool(false)));
    assert_eq!(
        transformed.get("model"),
        Some(&Value::String(String::from("grok-4")))
    );
}

#[test]
fn code_interpreter_keeps_only_its_type() {
    let transformed = mapped(&object([(
        "tools",
        Value::Array(vec![object([
            ("type", json!("code_interpreter")),
            ("container", object([("type", json!("auto"))])),
            ("extra", json!(1_i32)),
        ])]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![object([("type", json!("code_interpreter"))])]
    );
}

#[test]
fn nested_filters_win_over_flat_fields() {
    let transformed = mapped(&object([(
        "tools",
        Value::Array(vec![object([
            ("type", json!("web_search")),
            (
                "filters",
                object([
                    ("allowed_domains", json!(["nested"])),
                    ("excluded_domains", json!(["gone"])),
                ]),
            ),
            ("allowed_domains", json!(["flat"])),
            ("excluded_domains", json!(["flat-gone"])),
            ("search_context_size", json!("low")),
            ("enable_image_understanding", json!(true)),
            ("unknown", json!(1_i32)),
        ])]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![object([
            ("type", json!("web_search")),
            (
                "filters",
                object([
                    ("allowed_domains", json!(["nested"])),
                    ("excluded_domains", json!(["gone"])),
                ]),
            ),
            ("enable_image_understanding", json!(true)),
        ])]
    );
}

#[test]
fn flat_domains_fill_filters_without_a_mapping() {
    let transformed = mapped(&object([(
        "tools",
        Value::Array(vec![object([
            ("type", json!("web_search")),
            ("filters", Value::Null),
            ("allowed_domains", json!(["flat"])),
            ("excluded_domains", json!(["flat-gone"])),
        ])]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![object([
            ("type", json!("web_search")),
            (
                "filters",
                object([
                    ("allowed_domains", json!(["flat"])),
                    ("excluded_domains", json!(["flat-gone"])),
                ]),
            ),
        ])]
    );
}

#[test]
fn an_empty_filters_mapping_suppresses_flat_domains() {
    let transformed = mapped(&object([(
        "tools",
        Value::Array(vec![object([
            ("type", json!("web_search")),
            ("filters", json!({})),
            ("allowed_domains", json!(["flat"])),
        ])]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![object([("type", json!("web_search"))])]
    );
}

#[test]
fn a_filters_mapping_keeps_an_explicit_null_domain_list() {
    let transformed = mapped(&object([(
        "tools",
        Value::Array(vec![object([
            ("type", json!("web_search")),
            ("filters", object([("allowed_domains", Value::Null)])),
        ])]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![object([
            ("type", json!("web_search")),
            ("filters", object([("allowed_domains", Value::Null)])),
        ])]
    );
}

#[test]
fn x_search_keeps_every_handle_date_and_media_field() {
    let transformed = mapped(&object([(
        "tools",
        Value::Array(vec![object([
            ("type", json!("x_search")),
            ("allowed_x_handles", json!(["a"])),
            ("excluded_x_handles", Value::Null),
            ("from_date", json!("2026-01-01")),
            ("to_date", json!("2026-02-01")),
            ("enable_image_understanding", json!(false)),
            ("enable_video_understanding", Value::Null),
            ("unknown", json!("dropped")),
        ])]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![object([
            ("type", json!("x_search")),
            ("allowed_x_handles", json!(["a"])),
            ("excluded_x_handles", Value::Null),
            ("from_date", json!("2026-01-01")),
            ("to_date", json!("2026-02-01")),
            ("enable_image_understanding", json!(false)),
            ("enable_video_understanding", Value::Null),
        ])]
    );
}

// xAI's type hints say ten handles at most, but the code never counts them
#[test]
fn x_handles_are_not_capped_at_the_documented_ten() {
    let handles = (0..11_usize)
        .map(|index| format!("handle-{index}"))
        .collect::<Vec<String>>();
    let transformed = mapped(&object([(
        "tools",
        Value::Array(vec![object([
            ("type", json!("x_search")),
            ("allowed_x_handles", json!(handles)),
        ])]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![object([
            ("type", json!("x_search")),
            ("allowed_x_handles", json!(handles)),
        ])]
    );
}

#[test]
fn unknown_tools_and_non_objects_pass_through() {
    let function = object([
        ("type", json!("function")),
        ("name", json!("f")),
        ("parameters", json!({})),
    ]);
    let transformed = mapped(&object([(
        "tools",
        Value::Array(vec![function.clone(), json!("plain"), json!(3_i32)]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![function, json!("plain"), json!(3_i32)]
    );
}

#[test]
fn a_truthy_non_list_tools_value_becomes_one_element() {
    let transformed = mapped(&object([(
        "tools",
        object([
            ("type", json!("web_search")),
            ("allowed_domains", json!(["a"])),
        ]),
    )]));
    assert_eq!(
        tools(&transformed),
        &vec![object([
            ("type", json!("web_search")),
            ("filters", object([("allowed_domains", json!(["a"]))])),
        ])]
    );
}

#[rstest]
#[case::zero(json!(0_i32))]
#[case::empty_text(json!(""))]
#[case::empty_object(json!({}))]
#[case::empty_list(json!([]))]
#[case::false_value(json!(false))]
#[case::null_value(Value::Null)]
fn falsy_tools_are_not_normalized(#[case] given: Value) {
    let transformed = mapped(&object([("tools", given.clone())]));
    assert_eq!(transformed.get("tools"), Some(&given));
}

#[test]
fn absent_tools_and_store_stay_absent() {
    let transformed = mapped(&object([("model", json!("grok-4"))]));
    assert!(!transformed.contains_key("tools"));
    assert!(!transformed.contains_key("store"));
    let stored = mapped(&object([("store", json!(true))]));
    assert_eq!(stored.get("store"), Some(&Value::Bool(true)));
}

struct FixedOauth {
    fail: bool,
    calls: usize,
}

impl OauthAccess for FixedOauth {
    fn access_token(&mut self) -> impl Future<Output = Result<SecretString, XaiOauthError>> + Send {
        self.calls = self.calls.saturating_add(1);
        let outcome = if self.fail {
            Err(XaiOauthError::LoginRequired)
        } else {
            Ok(SecretString::from("oauth-token"))
        };
        ready(outcome)
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
    let mut login = oauth(false);
    let validated = validate_environment(Environment {
        headers: headers(&[("Authorization", "Bearer old"), ("X-Keep", "yes")]),
        api_key: Some(&key("explicit")),
        model: "grok-4",
        use_xai_oauth: false,
        settings: &empty,
        oauth: &mut login,
    })
    .await
    .unwrap();
    assert_eq!(&validated["Authorization"], "Bearer explicit");
    assert_eq!(&validated["X-Keep"], "yes");
    assert_eq!(login.calls, 0);
}

#[tokio::test]
async fn the_generic_key_falls_back_before_the_environment() {
    let both = settings(None, Some("generic"), Some("environment"), None);
    let mut first = oauth(false);
    let with_generic = validate_environment(Environment {
        headers: Headers::new(),
        api_key: None,
        model: "grok-4",
        use_xai_oauth: false,
        settings: &both,
        oauth: &mut first,
    })
    .await
    .unwrap();
    assert_eq!(&with_generic["Authorization"], "Bearer generic");
    assert_eq!(first.calls, 0);
    let environment_only = settings(None, None, Some("environment"), None);
    let mut second = oauth(false);
    let with_environment = validate_environment(Environment {
        headers: Headers::new(),
        api_key: None,
        model: "grok-4",
        use_xai_oauth: false,
        settings: &environment_only,
        oauth: &mut second,
    })
    .await
    .unwrap();
    assert_eq!(&with_environment["Authorization"], "Bearer environment");
}

#[tokio::test]
async fn a_module_key_beats_the_generic_and_environment_keys() {
    let configured = settings(Some("module"), Some("generic"), Some("environment"), None);
    let mut login = oauth(false);
    let validated = validate_environment(Environment {
        headers: Headers::new(),
        api_key: None,
        model: "grok-4",
        use_xai_oauth: false,
        settings: &configured,
        oauth: &mut login,
    })
    .await
    .unwrap();
    assert_eq!(&validated["Authorization"], "Bearer module");
    assert_eq!(login.calls, 0);
}

#[tokio::test]
async fn a_missing_key_raises_without_a_status() {
    let empty = settings(None, None, None, None);
    let mut login = oauth(false);
    let error = validate_environment(Environment {
        headers: Headers::new(),
        api_key: None,
        model: "grok-4",
        use_xai_oauth: false,
        settings: &empty,
        oauth: &mut login,
    })
    .await
    .unwrap_err();
    assert!(matches!(error, XaiResponsesError::MissingApiKey));
    assert_eq!(error.status_code(), None);
    assert_eq!(login.calls, 0);
}

#[tokio::test]
async fn an_opted_in_login_supplies_the_key_only_without_one() {
    let empty = settings(None, None, None, None);
    let mut login = oauth(false);
    let with_login = validate_environment(Environment {
        headers: Headers::new(),
        api_key: None,
        model: "grok-4",
        use_xai_oauth: true,
        settings: &empty,
        oauth: &mut login,
    })
    .await
    .unwrap();
    assert_eq!(&with_login["Authorization"], "Bearer oauth-token");
    assert_eq!(login.calls, 1);
    let mut explicit = oauth(false);
    let with_explicit = validate_environment(Environment {
        headers: Headers::new(),
        api_key: Some(&key("explicit")),
        model: "grok-4",
        use_xai_oauth: true,
        settings: &empty,
        oauth: &mut explicit,
    })
    .await
    .unwrap();
    assert_eq!(&with_explicit["Authorization"], "Bearer explicit");
    assert_eq!(explicit.calls, 0);
}

#[tokio::test]
async fn an_oauth_failure_is_an_authentication_error() {
    let empty = settings(None, None, None, None);
    let mut login = oauth(true);
    let error = validate_environment(Environment {
        headers: Headers::new(),
        api_key: None,
        model: "grok-4",
        use_xai_oauth: true,
        settings: &empty,
        oauth: &mut login,
    })
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
        get_complete_url(&UrlInput {
            api_base,
            litellm_api_base,
            api_key: None,
            use_xai_oauth: false,
            settings: &configured,
            oauth_base: "https://oauth.example/v1",
        }),
        expected
    );
}

#[test]
fn oauth_selects_its_own_base_only_without_a_key() {
    let without_key = settings(None, None, None, None);
    assert_eq!(
        get_complete_url(&UrlInput {
            api_base: Some("https://explicit.example/v1"),
            litellm_api_base: None,
            api_key: None,
            use_xai_oauth: true,
            settings: &without_key,
            oauth_base: "https://oauth.example/v1/",
        }),
        "https://oauth.example/v1/responses"
    );
    assert_eq!(
        get_complete_url(&UrlInput {
            api_base: Some("https://explicit.example/v1"),
            litellm_api_base: None,
            api_key: None,
            use_xai_oauth: false,
            settings: &without_key,
            oauth_base: "https://oauth.example/v1",
        }),
        "https://explicit.example/v1/responses"
    );
    let configured = settings(None, Some("generic"), Some("https://env.example/v1"), None);
    assert_eq!(
        get_complete_url(&UrlInput {
            api_base: Some("https://explicit.example/v1"),
            litellm_api_base: None,
            api_key: None,
            use_xai_oauth: true,
            settings: &configured,
            oauth_base: "https://oauth.example/v1",
        }),
        "https://explicit.example/v1/responses"
    );
}

#[test]
fn an_ordinary_reply_restates_the_reported_ticks() {
    let replied = restated(base_response(&object([
        ("input_tokens", json!(1_i32)),
        ("output_tokens", json!(2_i32)),
        ("total_tokens", json!(3_i32)),
        ("cost_in_usd_ticks", json!(12_500_000_000_i64)),
    ])));
    assert_eq!(cost(&replied), Some(Decimal::new(125, 2)));
}

#[rstest]
#[case::negative(json!(-1_i32))]
#[case::boolean(json!(true))]
#[case::fraction(json!(1.5_f64))]
#[case::text(json!("100"))]
#[case::absent(Value::Null)]
fn a_reply_without_valid_ticks_keeps_its_cost(#[case] given: Value) {
    let mut usage = object([
        ("input_tokens", json!(1_i32)),
        ("output_tokens", json!(2_i32)),
        ("total_tokens", json!(3_i32)),
        ("cost", json!(0.25_f64)),
    ]);
    if !given.is_null() {
        usage["cost_in_usd_ticks"] = given;
    }
    let replied = restated(base_response(&usage));
    assert_eq!(cost(&replied), Some(Decimal::new(25, 2)));
}

#[test]
fn a_reply_carrying_no_usage_stays_without_one() {
    let mut body = base_response(&json!({}));
    body.as_object_mut().unwrap().remove("usage");
    let replied = restated(body);
    assert_eq!(cost(&replied), None);
}

#[test]
fn a_lax_reply_is_left_as_sent() {
    let sent = ticks(object([("cost_in_usd_ticks", json!(12_500_000_000_i64))]));
    let mut reply = ResponsesReply {
        response: ReplyBody::Lax(IndexMap::from([(String::from("usage"), sent.clone())])),
        headers: IndexMap::new(),
        additional_headers: IndexMap::new(),
    };
    restate_reply_usage(&mut reply);
    let ReplyBody::Lax(fields) = reply.response else {
        panic!("a lax reply stays lax");
    };
    assert_eq!(fields.get("usage"), Some(&sent));
}

fn event(given: &Value) -> DecodedStreamResponse {
    let bytes = serde_json::to_vec(given).unwrap();
    decode_response_event(&bytes).unwrap()
}

fn terminal_event(event_type: &str) -> Value {
    object([
        ("type", json!(event_type)),
        (
            "response",
            base_response(&object([
                ("input_tokens", json!(1_i32)),
                ("output_tokens", json!(2_i32)),
                ("total_tokens", json!(3_i32)),
                ("cost_in_usd_ticks", json!(5_000_000_000_i64)),
            ])),
        ),
    ])
}

fn record_cost(decoded: &DecodedStreamResponse) -> Option<Decimal> {
    let DecodedStreamResponse::Known { record, .. } = decoded else {
        return None;
    };
    if let KnownResponseEvent::ResponseCompleted(fields)
    | KnownResponseEvent::ResponseIncomplete(fields)
    | KnownResponseEvent::ResponseFailed(fields) = record.as_ref()
    {
        return fields.response.usage.as_ref()?.cost;
    }
    if let KnownResponseEvent::ResponseCreated(fields)
    | KnownResponseEvent::ResponseInProgress(fields) = record.as_ref()
    {
        return fields.response.usage.as_ref()?.cost;
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
    let mut decoded = event(&terminal_event(event_type));
    restate_event_usage(&mut decoded);
    assert_eq!(record_cost(&decoded), expected);
}

#[test]
fn a_terminal_event_without_usage_is_left_alone() {
    let mut decoded = event(&object([
        ("type", json!("response.completed")),
        (
            "response",
            object([
                ("id", json!("resp-2")),
                ("created_at", json!(7_i32)),
                ("output", json!([])),
            ]),
        ),
    ]));
    restate_event_usage(&mut decoded);
    let DecodedStreamResponse::Known { record, .. } = decoded else {
        panic!("a valid terminal event decodes as known");
    };
    let KnownResponseEvent::ResponseCompleted(fields) = *record else {
        panic!("the completed event variant is the one that was decoded");
    };
    assert!(fields.response.usage.is_none());
}

#[test]
fn a_generic_event_is_left_alone() {
    let mut decoded = event(&object([
        ("type", json!("response.shell_call.in_progress")),
        ("extra", json!(1_i32)),
    ]));
    restate_event_usage(&mut decoded);
    assert!(matches!(decoded, DecodedStreamResponse::Generic(_)));
}

// LiteLLM raises AttributeError when a terminal payload fails its model and stays a dict
#[test]
fn a_lax_terminal_event_keeps_its_fields_as_sent() {
    let mut decoded = event(&object([
        ("type", json!("response.completed")),
        (
            "response",
            object([(
                "usage",
                object([("cost_in_usd_ticks", json!(5_000_000_000_i64))]),
            )]),
        ),
    ]));
    restate_event_usage(&mut decoded);
    let DecodedStreamResponse::Lax { fields, .. } = decoded else {
        panic!("an incomplete terminal payload decodes as lax");
    };
    assert_eq!(
        fields.get("response"),
        Some(&ticks(object([(
            "usage",
            object([("cost_in_usd_ticks", json!(5_000_000_000_i64))]),
        )])))
    );
}

#[test]
fn the_xai_responses_capability_is_not_native_websocket() {
    const {
        assert!(!SUPPORTS_NATIVE_WEBSOCKET);
    }
}

#[test]
fn a_restated_cost_keeps_the_ticks_beside_it() {
    let replied = restated(base_response(&object([
        ("input_tokens", json!(1_i32)),
        ("output_tokens", json!(2_i32)),
        ("total_tokens", json!(3_i32)),
        ("cost_in_usd_ticks", json!(1_i64)),
    ])));
    let usage = replied.usage.unwrap();
    assert_eq!(
        usage.extra.get("cost_in_usd_ticks"),
        Some(&ticks(json!(1_i64)))
    );
    assert_eq!(usage.cost, Some(Decimal::new(1, 10)));
}
