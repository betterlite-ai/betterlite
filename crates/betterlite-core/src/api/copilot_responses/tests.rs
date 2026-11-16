#![cfg(test)]

use std::future::{Future, ready};

use betterlite_config::sections::copilot_chat::Settings;
use betterlite_models::catalog::{ModelCatalog, UnknownFields};
use betterlite_models::decisions::Json;
use betterlite_models::full_model_metadata::HelperQuery;
use betterlite_models::full_model_metadata::record::ModelInfo;
use betterlite_openai::copilot::api_key::error::ApiKeyError;
use betterlite_openai::copilot::device_auth::error::DeviceAuthError;
use betterlite_openai::openai_base_config::Headers;
use betterlite_openai::response_events::event_decode::{
    DecodedStreamResponse, transform_streaming_response,
};
use betterlite_openai::response_input::ResponsesInput;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::value::RawValue;
use tokio::task::yield_now;
use uuid::Uuid;

use super::ResponsesConfig;
use super::error::CopilotResponsesError;
use super::input::{LiteLlmParams, has_vision_input, initiator, validated_input};
use super::model_gate::{MergedModelInfo, supports_responses_api};
use super::stream::StreamItemIds;
use crate::api::copilot_chat::credentials::CopilotCredentials;

const ADDED: &str =
    r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_1"}}"#;
const DELTA: &str =
    r#"{"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"other"}"#;
const DONE: &str = r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"other"}}"#;

#[derive(Debug)]
enum Outcome {
    Key(&'static str),
    NoToken,
    Refresh,
    Login,
}

struct Stored {
    outcome: Outcome,
    base: Option<&'static str>,
}

impl CopilotCredentials for Stored {
    fn api_base(&mut self) -> impl Future<Output = Option<String>> + Send {
        ready(self.base.map(String::from))
    }

    fn api_key(
        &mut self,
    ) -> impl Future<Output = Result<Option<SecretString>, ApiKeyError>> + Send {
        ready(match self.outcome {
            Outcome::Key(text) => Ok(Some(SecretString::from(String::from(text)))),
            Outcome::NoToken => Ok(None),
            Outcome::Refresh => Err(ApiKeyError::Refresh),
            Outcome::Login => Err(ApiKeyError::AccessToken(DeviceAuthError::ApiKey {
                status: 403,
            })),
        })
    }
}

fn fresh(outcome: Outcome, base: Option<&'static str>) -> ResponsesConfig<Stored> {
    ResponsesConfig {
        credentials: Stored { outcome, base },
        settings: Settings {
            disable_system_to_assistant: false,
            api_base: None,
        },
        max_recurse_depth: 100,
        item_ids: StreamItemIds::default(),
    }
}

fn input(text: &str) -> ResponsesInput {
    serde_json::from_str(text).expect("a Responses input")
}

fn chunk(text: &str) -> IndexMap<String, Json> {
    serde_json::from_str(text).expect("a stream chunk")
}

struct Scripted(Option<ModelInfo>, Vec<(String, Option<String>)>);

#[derive(Debug, thiserror::Error)]
#[error("model lookup failed")]
struct LookupFailed;

impl MergedModelInfo for Scripted {
    type Error = LookupFailed;

    fn merged_info(
        &mut self,
        query: &HelperQuery<'_>,
    ) -> impl Future<Output = Result<ModelInfo, LookupFailed>> + Send {
        self.1.push((
            query.model.to_owned(),
            query.custom_llm_provider.map(String::from),
        ));
        ready(self.0.clone().ok_or(LookupFailed))
    }
}

fn catalog(endpoints: &str) -> ModelCatalog {
    let entry = format!(
        r#"{{"github_copilot/codex": {{"litellm_provider": "github_copilot", "mode": "chat", "supported_endpoints": {endpoints}}}}}"#
    );
    ModelCatalog::parse(entry.as_bytes(), UnknownFields::Reject).expect("a catalog")
}

#[rstest]
#[case::mode_responses_accepts(
    Some(r#"{"mode":"responses","key":"github_copilot/codex"}"#),
    "[]",
    true
)]
#[case::mode_chat_rejects_a_dual_endpoint_model(
    Some(r#"{"mode":"chat","key":"github_copilot/codex"}"#),
    r#"["/v1/chat/completions","/v1/responses"]"#,
    false
)]
#[case::listed_endpoint_accepts(
    Some(r#"{"key":"github_copilot/codex"}"#),
    r#"["/v1/responses"]"#,
    true
)]
#[case::other_mode_reads_the_endpoints(
    Some(r#"{"mode":"completion","key":"github_copilot/codex"}"#),
    r#"["/v1/responses"]"#,
    true
)]
#[case::unlisted_endpoint_rejects(
    Some(r#"{"key":"github_copilot/codex"}"#),
    r#"["/v1/chat/completions"]"#,
    false
)]
#[case::key_missing_from_the_catalog(
    Some(r#"{"key":"github_copilot/absent"}"#),
    r#"["/v1/responses"]"#,
    false
)]
#[case::no_key_in_the_info(Some(r#"{"mode":"embedding"}"#), r#"["/v1/responses"]"#, false)]
#[case::lookup_failure(None, r#"["/v1/responses"]"#, false)]
#[tokio::test]
async fn model_gate_follows_mode_then_catalog_endpoints(
    #[case] info: Option<&str>,
    #[case] endpoints: &str,
    #[case] expected: bool,
) {
    let record = info.map(|text| {
        let raw = RawValue::from_string(text.to_owned()).expect("raw JSON");
        ModelInfo::parse(&raw).expect("model info")
    });
    let mut source = Scripted(record, Vec::new());
    let accepted = supports_responses_api("codex", &mut source, &catalog(endpoints)).await;
    assert_eq!(accepted, expected);
    assert_eq!(
        source.1,
        [(String::from("codex"), Some(String::from("github_copilot")))]
    );
}

#[rstest]
#[case::text_input("\"hello\"", "user")]
#[case::user_message(r#"[{"role":"user","content":"hi"}]"#, "user")]
#[case::tool_role_alone(r#"[{"role":"tool","content":"x"}]"#, "user")]
#[case::assistant_in_any_case(r#"[{"role":"user"},{"role":"AsSistant"}]"#, "agent")]
#[case::missing_role(r#"[{"role":"user"},{"type":"function_call_output"}]"#, "agent")]
#[case::empty_role(r#"[{"role":""}]"#, "agent")]
#[case::null_role(r#"[{"role":null}]"#, "agent")]
#[case::non_object_items_are_skipped(r#"["text",{"role":"user"}]"#, "user")]
#[case::zero_role(r#"[{"role":0}]"#, "agent")]
#[case::nonzero_number_role(r#"[{"role":7}]"#, "user")]
#[case::false_role(r#"[{"role":false}]"#, "agent")]
#[case::true_role(r#"[{"role":true}]"#, "user")]
#[case::empty_list_role(r#"[{"role":[]}]"#, "agent")]
#[case::filled_list_role(r#"[{"role":["x"]}]"#, "user")]
#[case::empty_object_role(r#"[{"role":{}}]"#, "agent")]
#[case::filled_object_role(r#"[{"role":{"a":1}}]"#, "user")]
fn initiator_marks_agent_traffic(#[case] text: &str, #[case] expected: &str) {
    assert_eq!(initiator(&input(text)), expected);
}

fn nested(levels: usize) -> String {
    format!(
        r#"{}{{"type":"input_image"}}{}"#,
        "[".repeat(levels),
        "]".repeat(levels)
    )
}

#[rstest]
#[case::image_part(
    r#"[{"role":"user","content":[{"type":"input_text"},{"type":"input_image"}]}]"#,
    100,
    true
)]
#[case::type_in_any_case(r#"[{"content":[{"type":"INPUT_Image"}]}]"#, 100, true)]
#[case::item_is_the_image(r#"[{"type":"input_image"}]"#, 100, true)]
#[case::content_that_is_not_a_list(r#"[{"content":"input_image"}]"#, 100, false)]
#[case::text_parts_only(r#"[{"content":[{"type":"input_text"}]}]"#, 100, false)]
#[case::text_input("\"input_image\"", 100, false)]
#[case::content_depth_inside_the_limit(r#"[{"content":[{"type":"input_image"}]}]"#, 2, true)]
#[case::content_depth_beyond_the_limit(r#"[{"content":[{"type":"input_image"}]}]"#, 1, false)]
fn vision_walk_follows_content_lists_up_to_the_depth(
    #[case] text: &str,
    #[case] max_depth: i64,
    #[case] expected: bool,
) {
    assert_eq!(has_vision_input(&input(text), max_depth), expected);
}

#[rstest]
#[case::at_the_default_limit(100, 100, true)]
#[case::beyond_the_default_limit(101, 100, false)]
#[case::zero_limit_sees_no_item(1, 0, false)]
#[case::negative_limit(1, -1, false)]
fn vision_depth_boundary_is_greater_than_the_limit(
    #[case] levels: usize,
    #[case] max_depth: i64,
    #[case] expected: bool,
) {
    assert_eq!(
        has_vision_input(&input(&nested(levels)), max_depth),
        expected
    );
}

#[rstest]
#[case::null_fields_go(
    r#"[{"type":"reasoning","id":"r","status":null,"summary":[],"content":null}]"#,
    r#"[{"type":"reasoning","id":"r","summary":[]}]"#
)]
#[case::encrypted_content_stays(
    r#"[{"type":"reasoning","id":"r","encrypted_content":"abc","status":null}]"#,
    r#"[{"type":"reasoning","id":"r","encrypted_content":"abc"}]"#
)]
#[case::null_encrypted_content_goes(
    r#"[{"type":"reasoning","encrypted_content":null}]"#,
    r#"[{"type":"reasoning"}]"#
)]
#[case::other_items_keep_their_nulls(
    r#"[{"type":"message","content":null}]"#,
    r#"[{"type":"message","content":null}]"#
)]
#[case::untyped_item_is_unchanged(r#"[{"id":null}]"#, r#"[{"id":null}]"#)]
#[case::non_object_items_pass(r#"["x",3,null]"#, r#"["x",3,null]"#)]
#[case::text_input("\"hello\"", "\"hello\"")]
fn reasoning_items_lose_every_null_field(#[case] text: &str, #[case] expected: &str) {
    let validated = serde_json::to_string(&validated_input(&input(text))).expect("input JSON");
    assert_eq!(validated, expected);
}

#[rstest]
#[case::sub_event_takes_the_anchor(&[ADDED, DELTA], r#"{"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"rs_1"}"#)]
#[case::done_without_item_id_rewrites_the_nested_id(&[ADDED, DONE], r#"{"type":"response.output_item.done","output_index":0,"item":{"id":"rs_1"}}"#)]
#[case::done_with_both_keeps_the_nested_id(&[ADDED, r#"{"type":"response.output_item.done","output_index":0,"item_id":"x","item":{"id":"nested"}}"#], r#"{"type":"response.output_item.done","output_index":0,"item_id":"rs_1","item":{"id":"nested"}}"#)]
#[case::null_item_id_falls_to_the_nested_id(&[ADDED, r#"{"type":"response.output_item.done","output_index":0,"item_id":null,"item":{"id":"x"}}"#], r#"{"type":"response.output_item.done","output_index":0,"item_id":null,"item":{"id":"rs_1"}}"#)]
#[case::event_before_the_anchor_passes(&[DELTA], DELTA)]
#[case::added_event_is_unchanged(&[ADDED], ADDED)]
#[case::repeated_added_replaces_the_anchor(&[ADDED, r#"{"type":"response.output_item.added","output_index":0,"item":{"id":"rs_2"}}"#, DELTA], r#"{"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":"rs_2"}"#)]
#[case::empty_id_anchors(&[r#"{"type":"response.output_item.added","output_index":0,"item":{"id":""}}"#, DELTA], r#"{"type":"response.reasoning_summary_text.delta","output_index":0,"item_id":""}"#)]
#[case::non_string_id_does_not_anchor(&[r#"{"type":"response.output_item.added","output_index":0,"item":{"id":5}}"#, DELTA], DELTA)]
#[case::another_index_is_untouched(&[ADDED, r#"{"type":"x.delta","output_index":1,"item_id":"o"}"#], r#"{"type":"x.delta","output_index":1,"item_id":"o"}"#)]
#[case::bool_index_is_an_integer(&[r#"{"type":"response.output_item.added","output_index":true,"item":{"id":"rs_1"}}"#, r#"{"type":"x.delta","output_index":1,"item_id":"o"}"#], r#"{"type":"x.delta","output_index":1,"item_id":"rs_1"}"#)]
#[case::false_index_is_zero(&[ADDED, r#"{"type":"x.delta","output_index":false,"item_id":"o"}"#], r#"{"type":"x.delta","output_index":false,"item_id":"rs_1"}"#)]
#[case::float_index_is_not_an_integer(&[ADDED, r#"{"type":"x.delta","output_index":0.0,"item_id":"o"}"#], r#"{"type":"x.delta","output_index":0.0,"item_id":"o"}"#)]
#[case::text_index_is_not_an_integer(&[ADDED, r#"{"type":"x.delta","output_index":"0","item_id":"o"}"#], r#"{"type":"x.delta","output_index":"0","item_id":"o"}"#)]
#[case::missing_index_passes(&[ADDED, r#"{"type":"x.delta","item_id":"o"}"#], r#"{"type":"x.delta","item_id":"o"}"#)]
#[case::done_without_an_item_object_passes(&[ADDED, r#"{"type":"response.output_item.done","output_index":0,"item":"x"}"#], r#"{"type":"response.output_item.done","output_index":0,"item":"x"}"#)]
fn stream_events_take_the_anchored_item_id(#[case] events: &[&str], #[case] expected: &str) {
    let mut ids = StreamItemIds::default();
    let mut last = IndexMap::new();
    for event in events {
        last = ids.normalize(chunk(event));
    }
    assert_eq!(serde_json::to_string(&last).expect("chunk JSON"), expected);
}

async fn replay(anchor: &str) -> Option<Json> {
    let mut responses = fresh(Outcome::NoToken, None);
    let added = format!(
        r#"{{"type":"response.output_item.added","output_index":0,"item":{{"id":"{anchor}"}}}}"#
    );
    drop(responses.item_ids.normalize(chunk(&added)));
    yield_now().await;
    responses
        .item_ids
        .normalize(chunk(DELTA))
        .get("item_id")
        .cloned()
}

#[tokio::test]
async fn concurrent_streams_with_the_same_indices_keep_their_own_anchors() {
    let (left, right) = tokio::join!(replay("rs_left"), replay("rs_right"));
    assert_eq!(left, Some(Json::Text(String::from("rs_left"))));
    assert_eq!(right, Some(Json::Text(String::from("rs_right"))));
}

#[test]
fn the_rewrite_runs_before_the_event_is_decoded() {
    let mut responses = fresh(Outcome::NoToken, None);
    drop(responses.item_ids.normalize(chunk(ADDED)));
    let vendor = r#"{"type":"x.delta","output_index":0,"item_id":"other"}"#;
    let decoded = transform_streaming_response(&responses.item_ids.normalize(chunk(vendor)));
    let DecodedStreamResponse::Generic(event) = decoded else {
        panic!("an unknown event type decodes as a generic event");
    };
    assert_eq!(
        event.extra.get("item_id"),
        Some(&Json::Text(String::from("rs_1")))
    );
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| (String::from(*name), String::from(*value)))
        .collect()
}

#[tokio::test]
async fn caller_headers_win_and_typed_input_sets_the_initiator_and_vision_headers() {
    let items =
        input(r#"[{"role":"user","content":[{"type":"input_image"}]},{"role":"assistant"}]"#);
    let params = LiteLlmParams::Typed {
        input: Some(&items),
    };
    let given = headers(&[("editor-version", "mine"), ("X-Initiator", "user")]);
    let merged = fresh(Outcome::Key("tid=1"), None)
        .validate_environment(given, &params, "gpt-5", Uuid::from_u128(7))
        .await
        .expect("headers");
    assert_eq!(merged["Authorization"], "Bearer tid=1");
    assert_eq!(merged["editor-version"], "mine");
    assert_eq!(merged["copilot-integration-id"], "vscode-chat");
    assert_eq!(merged["X-Initiator"], "agent");
    assert_eq!(merged["copilot-vision-request"], "true");
}

#[rstest]
#[case::plain_map(LiteLlmParams::Map)]
#[case::typed_without_input(LiteLlmParams::Typed { input: None })]
#[tokio::test]
async fn only_the_typed_input_attribute_is_classified(#[case] params: LiteLlmParams<'static>) {
    let merged = fresh(Outcome::Key("tid=1"), None)
        .validate_environment(Headers::new(), &params, "gpt-5", Uuid::from_u128(7))
        .await
        .expect("headers");
    assert!(!merged.contains_key("X-Initiator"));
    assert!(!merged.contains_key("copilot-vision-request"));
}

#[rstest]
#[case::no_token(Outcome::NoToken, 401, "required")]
#[case::empty_token(Outcome::Key(""), 401, "required")]
#[case::refresh_failure(Outcome::Refresh, 401, "authentication")]
#[case::login_failure(Outcome::Login, 403, "login")]
#[tokio::test]
async fn a_missing_subscription_key_is_an_authentication_error(
    #[case] outcome: Outcome,
    #[case] status: u16,
    #[case] kind: &str,
) {
    let error = fresh(outcome, None)
        .validate_environment(
            Headers::new(),
            &LiteLlmParams::Map,
            "gpt-5",
            Uuid::from_u128(7),
        )
        .await
        .expect_err("no key");
    assert_eq!(error.status(), status);
    let matched = match error {
        CopilotResponsesError::MissingKey { .. } => "required",
        CopilotResponsesError::Authentication { .. } => "authentication",
        CopilotResponsesError::Login(_) => "login",
    };
    assert_eq!(matched, kind);
}

#[rstest]
#[case::explicit_wins(
    Some("https://explicit.test//"),
    Some("https://cached.test"),
    Some("https://env.test"),
    "https://explicit.test/responses"
)]
#[case::cached_endpoint_beats_the_environment(
    None,
    Some("https://cached.test/"),
    Some("https://env.test"),
    "https://cached.test/responses"
)]
#[case::environment_beats_the_default(
    None,
    None,
    Some("https://env.test"),
    "https://env.test/responses"
)]
#[case::default_base(None, None, None, "https://api.githubcopilot.com/responses")]
#[tokio::test]
async fn the_responses_url_follows_the_base_precedence(
    #[case] explicit: Option<&str>,
    #[case] cached: Option<&'static str>,
    #[case] environment: Option<&str>,
    #[case] expected: &str,
) {
    let mut responses = fresh(Outcome::NoToken, cached);
    responses.settings.api_base = environment.map(String::from);
    assert_eq!(responses.get_complete_url(explicit).await, expected);
}
