#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::sync::Mutex;
use std::time::Duration;

use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;

use crate::agent_launch::EnvVars;

use super::{
    ListedModel, MAX_LISTING_BODY_BYTES, ModelListingRequest, ModelListingResponse,
    ModelListingTransport, ModelSyncArgs, ModelSyncResult, ModelSyncSkipped,
    OPENCODE_CONFIG_CONTENT_ENV, chat_models, fetch_model_listing, opencode_model_entry,
    opencode_model_sync_env, opencode_provider_config,
};

const KEY: &str = "sk-test";

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedRequest {
    url: String,
    authorization: String,
    timeout: Duration,
    max_body_bytes: usize,
}

enum Reply {
    Status(u16, Vec<u8>),
    Failure(io::ErrorKind),
}

struct FakeTransport {
    requests: Mutex<Vec<RecordedRequest>>,
    reply: Reply,
}

impl FakeTransport {
    fn replying(status: u16, body: &str) -> Self {
        Self::replying_bytes(status, body.as_bytes().to_vec())
    }

    fn replying_bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            reply: Reply::Status(status, body),
        }
    }

    fn failing(kind: io::ErrorKind) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            reply: Reply::Failure(kind),
        }
    }

    fn requests(&self) -> Vec<RecordedRequest> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelListingTransport for FakeTransport {
    fn get(&self, request: &ModelListingRequest) -> io::Result<ModelListingResponse> {
        self.requests
            .lock()
            .map_err(|_poisoned| io::Error::other("the request log lock is poisoned"))?
            .push(RecordedRequest {
                url: request.url.clone(),
                authorization: request.authorization.expose_secret().to_owned(),
                timeout: request.timeout,
                max_body_bytes: request.max_body_bytes,
            });
        match &self.reply {
            Reply::Status(status, body) => Ok(ModelListingResponse {
                status: *status,
                body: body.clone(),
            }),
            Reply::Failure(kind) => Err(io::Error::from(*kind)),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ExpectedConfig {
    provider: BTreeMap<String, ExpectedProvider>,
}

#[derive(Debug, Deserialize)]
struct ExpectedProvider {
    npm: String,
    name: String,
    options: ExpectedOptions,
    models: BTreeMap<String, ExpectedModel>,
}

#[derive(Debug, Deserialize)]
struct ExpectedOptions {
    #[serde(rename = "baseURL")]
    base_url: String,
    #[serde(rename = "apiKey")]
    api_key: String,
}

#[derive(Debug, Deserialize)]
struct ExpectedModel {
    name: String,
    limit: Option<ExpectedLimit>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct ExpectedLimit {
    context: i64,
    output: i64,
}

fn key() -> SecretString {
    SecretString::from(KEY)
}

fn listed(id: &str, mode: Option<&str>, limits: Option<(i64, i64)>) -> ListedModel {
    ListedModel {
        id: id.to_owned(),
        mode: mode.map(str::to_owned),
        max_input_tokens: limits.map(|(input, _output)| input),
        max_output_tokens: limits.map(|(_input, output)| output),
    }
}

fn config_for(base_url: &str, models: &[ListedModel]) -> ExpectedConfig {
    let json = opencode_provider_config(base_url, models).expect("the OpenCode config serializes");
    serde_json::from_str(&json).expect("the OpenCode config is JSON")
}

fn env_of(result: ModelSyncResult) -> EnvVars {
    match result {
        ModelSyncResult::Env(env) => env,
        ModelSyncResult::Args(_) | ModelSyncResult::Skipped(_) => {
            panic!("expected an env result")
        }
    }
}

fn skip_of(result: ModelSyncResult) -> ModelSyncSkipped {
    match result {
        ModelSyncResult::Skipped(skipped) => skipped,
        ModelSyncResult::Env(_) | ModelSyncResult::Args(_) => panic!("expected a skip"),
    }
}

fn model_ids(models: &[ListedModel]) -> Vec<String> {
    models.iter().map(|model| model.id.clone()).collect()
}

#[test]
fn chat_mode_keeps_models_without_a_mode() {
    let models = vec![
        listed("plain", None, None),
        listed("embed", Some("embedding"), None),
    ];
    let kept = chat_models(&models);
    assert_eq!(model_ids(&kept), ["plain"]);
}

#[rstest]
#[case::chat("chat")]
#[case::responses("responses")]
fn chat_mode_keeps_chat_and_responses(#[case] mode: &str) {
    let models = vec![listed("model", Some(mode), None)];
    assert_eq!(model_ids(&chat_models(&models)), ["model"]);
}

#[rstest]
#[case::completion("completion")]
#[case::embedding("embedding")]
#[case::image_generation("image_generation")]
#[case::moderation("moderation")]
#[case::rerank("rerank")]
#[case::audio_speech("audio_speech")]
#[case::empty_mode("")]
#[case::uppercase_chat("CHAT")]
fn chat_mode_drops_every_other_mode(#[case] mode: &str) {
    let models = vec![listed("model", Some(mode), None)];
    assert!(chat_models(&models).is_empty());
}

#[test]
fn chat_mode_keeps_the_listing_order() {
    let models = vec![
        listed("first", Some("chat"), None),
        listed("second", None, Some((1000, 500))),
        listed("third", Some("responses"), None),
    ];
    assert_eq!(
        model_ids(&chat_models(&models)),
        ["first", "second", "third"]
    );
}

#[test]
fn chat_mode_of_an_empty_listing_is_empty() {
    assert!(chat_models(&[]).is_empty());
}

#[test]
fn fetch_requests_models_with_bearer_auth_and_ten_second_timeout() {
    let transport = FakeTransport::replying(200, r#"{"data":[]}"#);
    let listing = fetch_model_listing("http://proxy:4000", &key(), &transport).expect("listing");
    assert!(listing.is_empty());
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://proxy:4000/v1/models");
    assert_eq!(requests[0].authorization, "Bearer sk-test");
    assert_eq!(requests[0].timeout, Duration::from_secs(10));
    assert_eq!(requests[0].max_body_bytes, MAX_LISTING_BODY_BYTES);
}

#[test]
fn fetch_body_limit_is_eight_mebibytes() {
    assert_eq!(MAX_LISTING_BODY_BYTES, 0x0080_0000);
}

#[test]
fn fetch_skips_a_body_over_the_size_limit() {
    let transport = FakeTransport::replying_bytes(200, vec![b' '; MAX_LISTING_BODY_BYTES + 1]);
    let skipped = fetch_model_listing("http://proxy", &key(), &transport)
        .expect_err("an oversized body is a skip");
    assert_eq!(
        skipped.reason,
        format!("http://proxy/v1/models returned more than {MAX_LISTING_BODY_BYTES} bytes")
    );
}

#[test]
fn fetch_allows_a_body_at_the_size_limit() {
    let transport = FakeTransport::replying_bytes(200, vec![b' '; MAX_LISTING_BODY_BYTES]);
    let skipped = fetch_model_listing("http://proxy", &key(), &transport)
        .expect_err("whitespace is not a listing");
    assert_eq!(
        skipped.reason,
        "http://proxy/v1/models returned an unexpected body"
    );
}

#[rstest]
#[case::plain("http://proxy:4000", "http://proxy:4000/v1/models")]
#[case::one_slash("http://proxy:4000/", "http://proxy:4000/v1/models")]
#[case::two_slashes("http://proxy:4000//", "http://proxy:4000/v1/models")]
#[case::only_slash("/", "/v1/models")]
#[case::empty("", "/v1/models")]
fn fetch_uses_the_base_url_without_trailing_slashes(
    #[case] base_url: &str,
    #[case] expected: &str,
) {
    let transport = FakeTransport::replying(200, r#"{"data":[]}"#);
    fetch_model_listing(base_url, &key(), &transport).expect("listing");
    assert_eq!(transport.requests()[0].url, expected);
}

#[test]
fn fetch_returns_the_listed_models() {
    let body = r#"{"data":[{"id":"gpt","object":"model","created":7,"owned_by":"openai","mode":"chat","max_input_tokens":128000,"max_output_tokens":32000}]}"#;
    let transport = FakeTransport::replying(200, body);
    let listing = fetch_model_listing("http://proxy", &key(), &transport).expect("listing");
    assert_eq!(
        listing,
        vec![listed("gpt", Some("chat"), Some((128_000, 32_000)))]
    );
}

#[rstest]
#[case::absent(r#"{"data":[{"id":"m"}]}"#)]
#[case::null(
    r#"{"data":[{"id":"m","mode":null,"max_input_tokens":null,"max_output_tokens":null}]}"#
)]
fn fetch_keeps_absent_fields_as_none(#[case] body: &str) {
    let transport = FakeTransport::replying(200, body);
    let listing = fetch_model_listing("http://proxy", &key(), &transport).expect("listing");
    assert_eq!(listing, vec![listed("m", None, None)]);
}

#[test]
fn fetch_ignores_unknown_fields() {
    let body = r#"{"object":"list","total":1,"data":[{"id":"m","created":7,"owned_by":"proxy","extra":true}]}"#;
    let transport = FakeTransport::replying(200, body);
    let listing = fetch_model_listing("http://proxy", &key(), &transport).expect("listing");
    assert_eq!(listing, vec![listed("m", None, None)]);
}

#[test]
fn fetch_skips_when_the_proxy_is_unreachable() {
    let transport = FakeTransport::failing(io::ErrorKind::ConnectionRefused);
    let skipped =
        fetch_model_listing("http://proxy", &key(), &transport).expect_err("unreachable is a skip");
    assert_eq!(
        skipped.reason,
        "could not reach http://proxy/v1/models: connection refused"
    );
}

#[rstest]
#[case::created(201)]
#[case::no_content(204)]
#[case::bad_request(400)]
#[case::unauthorized(401)]
#[case::server_error(500)]
fn fetch_skips_every_status_but_200(#[case] status: u16) {
    let transport = FakeTransport::replying(status, r#"{"data":[]}"#);
    let skipped =
        fetch_model_listing("http://proxy", &key(), &transport).expect_err("non-200 is a skip");
    assert_eq!(
        skipped.reason,
        format!("http://proxy/v1/models returned HTTP {status}")
    );
}

#[rstest]
#[case::not_json("not json")]
#[case::empty("")]
#[case::top_level_array("[]")]
#[case::missing_data(r#"{"models":[]}"#)]
#[case::data_object(r#"{"data":{}}"#)]
#[case::missing_id(r#"{"data":[{"mode":"chat"}]}"#)]
#[case::id_not_a_string(r#"{"data":[{"id":7}]}"#)]
#[case::mode_not_a_string(r#"{"data":[{"id":"m","mode":5}]}"#)]
#[case::limit_not_a_number(r#"{"data":[{"id":"m","max_input_tokens":"128"}]}"#)]
fn fetch_skips_a_body_it_cannot_decode(#[case] body: &str) {
    let transport = FakeTransport::replying(200, body);
    let skipped =
        fetch_model_listing("http://proxy", &key(), &transport).expect_err("bad body is a skip");
    assert_eq!(
        skipped.reason,
        "http://proxy/v1/models returned an unexpected body"
    );
}

#[test]
fn model_entry_maps_both_limits() {
    let entry = opencode_model_entry(&listed("m", Some("chat"), Some((128_000, 32_000))));
    assert_eq!(entry.name, "m");
    assert_eq!(
        entry.limit.map(|limit| (limit.context, limit.output)),
        Some((128_000, 32_000))
    );
}

#[rstest]
#[case::input_missing(None, Some(32_000))]
#[case::output_missing(Some(128_000), None)]
#[case::both_missing(None, None)]
fn model_entry_omits_limit_when_incomplete(
    #[case] input: Option<i64>,
    #[case] output: Option<i64>,
) {
    let model = ListedModel {
        id: "m".to_owned(),
        mode: Some("chat".to_owned()),
        max_input_tokens: input,
        max_output_tokens: output,
    };
    let entry = opencode_model_entry(&model);
    assert_eq!(entry.name, "m");
    assert!(entry.limit.is_none());
}

#[test]
fn model_entry_maps_zero_limits() {
    let entry = opencode_model_entry(&listed("m", Some("chat"), Some((0, 0))));
    assert_eq!(
        entry.limit.map(|limit| (limit.context, limit.output)),
        Some((0, 0))
    );
}

#[test]
fn model_entry_names_the_model_by_its_id() {
    let entry = opencode_model_entry(&listed("proxy-alias", Some("responses"), None));
    assert_eq!(entry.name, "proxy-alias");
}

#[test]
fn provider_config_declares_the_litellm_provider() {
    let config = config_for("http://proxy", &[listed("chat", Some("chat"), None)]);
    assert_eq!(config.provider.len(), 1);
    let provider = &config.provider["litellm"];
    assert_eq!(provider.npm, "@ai-sdk/openai-compatible");
    assert_eq!(provider.name, "LiteLLM");
}

#[rstest]
#[case::plain("http://proxy:4000", "http://proxy:4000/v1")]
#[case::trailing_slash("http://proxy:4000/", "http://proxy:4000/v1")]
#[case::empty("", "/v1")]
fn provider_config_points_at_the_proxy_v1(#[case] base_url: &str, #[case] expected: &str) {
    let config = config_for(base_url, &[]);
    assert_eq!(config.provider["litellm"].options.base_url, expected);
}

#[test]
fn provider_config_reads_the_key_from_the_openai_env() {
    let config = config_for("http://proxy", &[]);
    assert_eq!(
        config.provider["litellm"].options.api_key,
        "{env:OPENAI_API_KEY}"
    );
}

#[test]
fn provider_config_lists_only_chat_models() {
    let models = vec![
        listed("chat", Some("chat"), None),
        listed("responses", Some("responses"), None),
        listed("plain", None, None),
        listed("embedding", Some("embedding"), None),
    ];
    let config = config_for("http://proxy", &models);
    let names: Vec<&str> = config.provider["litellm"]
        .models
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(names, ["chat", "plain", "responses"]);
}

#[test]
fn provider_config_maps_limits_for_complete_models() {
    let config = config_for(
        "http://proxy",
        &[listed("m", Some("chat"), Some((128_000, 32_000)))],
    );
    let model = &config.provider["litellm"].models["m"];
    assert_eq!(model.name, "m");
    assert_eq!(
        model.limit,
        Some(ExpectedLimit {
            context: 128_000,
            output: 32_000,
        })
    );
}

#[test]
fn provider_config_omits_limit_when_incomplete() {
    let model = ListedModel {
        id: "half".to_owned(),
        mode: Some("chat".to_owned()),
        max_input_tokens: None,
        max_output_tokens: Some(32_000),
    };
    let json = opencode_provider_config("http://proxy", &[model]).expect("the config serializes");
    assert!(!json.contains("\"limit\""));
    let config: ExpectedConfig = serde_json::from_str(&json).expect("the config is JSON");
    assert_eq!(config.provider["litellm"].models["half"].name, "half");
}

#[test]
fn provider_config_with_no_models_has_an_empty_map() {
    let config = config_for("http://proxy", &[]);
    assert!(config.provider["litellm"].models.is_empty());
}

#[test]
fn provider_config_keeps_the_last_entry_for_a_duplicate_id() {
    let models = vec![
        listed("dup", Some("chat"), Some((1000, 500))),
        listed("dup", Some("chat"), None),
    ];
    let config = config_for("http://proxy", &models);
    assert_eq!(config.provider["litellm"].models.len(), 1);
    assert!(config.provider["litellm"].models["dup"].limit.is_none());
}

#[test]
fn model_sync_env_returns_the_config_content_env() {
    let body =
        r#"{"data":[{"id":"chat","mode":"chat","max_input_tokens":1000,"max_output_tokens":500}]}"#;
    let transport = FakeTransport::replying(200, body);
    let env = env_of(opencode_model_sync_env(
        &EnvVars::new(),
        "http://proxy",
        &key(),
        &transport,
    ));
    assert_eq!(env.len(), 1);
    let json = env
        .get(OPENCODE_CONFIG_CONTENT_ENV)
        .expect("config content");
    let config: ExpectedConfig = serde_json::from_str(json).expect("the config is JSON");
    assert_eq!(config.provider["litellm"].models["chat"].name, "chat");
}

#[rstest]
#[case::set("a stale config")]
#[case::empty("")]
fn model_sync_env_skips_when_config_content_is_set(#[case] value: &str) {
    let mut base = EnvVars::new();
    base.insert(OPENCODE_CONFIG_CONTENT_ENV.to_owned(), value.to_owned());
    let transport = FakeTransport::replying(200, r#"{"data":[]}"#);
    let skipped = skip_of(opencode_model_sync_env(
        &base,
        "http://proxy",
        &key(),
        &transport,
    ));
    assert_eq!(skipped.reason, "OPENCODE_CONFIG_CONTENT is already set");
    assert!(transport.requests().is_empty());
}

#[test]
fn model_sync_env_skips_on_a_failed_fetch() {
    let transport = FakeTransport::failing(io::ErrorKind::TimedOut);
    let skipped = skip_of(opencode_model_sync_env(
        &EnvVars::new(),
        "http://proxy",
        &key(),
        &transport,
    ));
    assert!(
        skipped
            .reason
            .starts_with("could not reach http://proxy/v1/models: ")
    );
}

#[test]
fn model_sync_env_skips_on_a_non_200_listing() {
    let transport = FakeTransport::replying(503, "");
    let skipped = skip_of(opencode_model_sync_env(
        &EnvVars::new(),
        "http://proxy",
        &key(),
        &transport,
    ));
    assert_eq!(skipped.reason, "http://proxy/v1/models returned HTTP 503");
}

#[test]
fn model_sync_env_passes_the_base_url_and_key_to_the_fetch() {
    let body =
        r#"{"data":[{"id":"chat","mode":"chat","max_input_tokens":1000,"max_output_tokens":500}]}"#;
    let transport = FakeTransport::replying(200, body);
    let mut base = EnvVars::new();
    base.insert("OPENCODE_CONFIG".to_owned(), "user-config".to_owned());
    let env = env_of(opencode_model_sync_env(
        &base,
        "http://proxy:4000/",
        &key(),
        &transport,
    ));
    assert_eq!(env.len(), 1);
    let json = env
        .get(OPENCODE_CONFIG_CONTENT_ENV)
        .expect("config content");
    let config: ExpectedConfig = serde_json::from_str(json).expect("the config is JSON");
    assert_eq!(
        config.provider["litellm"].options.base_url,
        "http://proxy:4000/v1"
    );
    assert_eq!(
        config.provider["litellm"].options.api_key,
        "{env:OPENAI_API_KEY}"
    );
    assert_eq!(
        config.provider["litellm"].models["chat"].limit,
        Some(ExpectedLimit {
            context: 1000,
            output: 500,
        })
    );
    let requests = transport.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].url, "http://proxy:4000/v1/models");
    assert_eq!(requests[0].authorization, "Bearer sk-test");
}

#[test]
fn contracts_listed_model_exposes_its_fields() {
    let model = ListedModel {
        id: "model".to_owned(),
        mode: Some("chat".to_owned()),
        max_input_tokens: Some(128_000),
        max_output_tokens: Some(32_000),
    };
    assert_eq!(model.clone(), model);
    assert_eq!(model.id, "model");
    assert_eq!(model.mode.as_deref(), Some("chat"));
    assert_eq!(model.max_input_tokens, Some(128_000));
    assert_eq!(model.max_output_tokens, Some(32_000));
}

#[test]
fn contracts_model_sync_args_carry_the_arguments() {
    let args = ModelSyncArgs {
        args: vec!["-c".to_owned(), "model_provider=\"litellm\"".to_owned()],
    };
    assert_eq!(args.args, ["-c", "model_provider=\"litellm\""]);
}

#[test]
fn contracts_model_sync_skipped_carries_the_reason() {
    let skipped = ModelSyncSkipped {
        reason: "OPENCODE_CONFIG_CONTENT is already set".to_owned(),
    };
    assert_eq!(skipped.reason, "OPENCODE_CONFIG_CONTENT is already set");
}

#[test]
fn contracts_model_sync_result_has_env_args_and_skipped() {
    let env = ModelSyncResult::Env(EnvVars::from([("A".to_owned(), "1".to_owned())]));
    let args = ModelSyncResult::Args(ModelSyncArgs {
        args: vec!["--model".to_owned()],
    });
    let skipped = ModelSyncResult::Skipped(ModelSyncSkipped {
        reason: "skip".to_owned(),
    });
    assert!(matches!(env, ModelSyncResult::Env(_)));
    assert!(matches!(args, ModelSyncResult::Args(_)));
    assert!(matches!(skipped, ModelSyncResult::Skipped(_)));
    assert_eq!(
        ModelSyncResult::Skipped(ModelSyncSkipped {
            reason: "skip".to_owned(),
        }),
        skipped
    );
}
