#![cfg(test)]

use std::collections::BTreeMap;
use std::fs;
use std::future::{Future, ready};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use indexmap::IndexMap;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};
use tokio::runtime::{Builder, Runtime};

use crate::decisions::Json;
use crate::fetch_policy::response::FetchResponse;
use crate::lemonade_model_metadata::{
    DEFAULT_API_BASE, DEFAULT_API_KEY, DeploymentCredentials, LemonadeCredentials,
    LemonadeModelInfo, ModelInfoClient, ModelInfoReplyError, ModelInfoRequest, auth_header,
    context_window, decode_lemonade_model_reply, get_model_info, positive_int,
    provider_specific_entry, resolve_origin,
};

#[rstest]
#[case::configured_origin_auth(configured_origin_auth)]
#[case::positive_token_limits(positive_token_limits)]
#[case::provider_metadata_overlay(provider_metadata_overlay)]
#[case::model_query_and_defaults(model_query_and_defaults)]
#[case::model_limit_projection(model_limit_projection)]
fn lemonade_model_metadata_cases(#[case] scenario: fn()) {
    scenario();
}

struct Fixture {
    reply: Option<(u16, Vec<u8>)>,
    requests: Mutex<Vec<ModelInfoRequest>>,
}

impl Fixture {
    fn answering(status: u16, body: &str) -> Self {
        Self {
            reply: Some((status, body.as_bytes().to_vec())),
            requests: Mutex::default(),
        }
    }

    fn failing() -> Self {
        Self {
            reply: None,
            requests: Mutex::default(),
        }
    }

    fn requests(&self) -> Vec<ModelInfoRequest> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

#[derive(Debug, thiserror::Error)]
#[error("the fixture transport failed")]
struct FixtureError;

impl ModelInfoClient for Fixture {
    type Error = FixtureError;

    fn get(
        &self,
        request: &ModelInfoRequest,
    ) -> impl Future<Output = Result<FetchResponse, Self::Error>> + Send {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        let reply = self.reply.clone();
        ready(match reply {
            Some((status, body)) => Ok(FetchResponse {
                status,
                headers: BTreeMap::new(),
                body,
            }),
            None => Err(FixtureError),
        })
    }
}

// LiteLLM's request_timeout default is 6000 seconds
const REQUEST_TIMEOUT: Duration = Duration::from_mins(100);

fn runtime() -> Runtime {
    Builder::new_current_thread().enable_all().build().unwrap()
}

fn credentials(
    api_base: Option<&str>,
    api_key: Option<&str>,
    lemonade_key: Option<&str>,
) -> LemonadeCredentials {
    LemonadeCredentials {
        api_base: api_base.map(str::to_owned),
        api_key: api_key.map(SecretString::from),
        lemonade_key: lemonade_key.map(SecretString::from),
    }
}

fn lookup(
    client: &Fixture,
    settings: &LemonadeCredentials,
    model: &str,
    deployment: DeploymentCredentials<'_>,
) -> Result<LemonadeModelInfo, ModelInfoReplyError> {
    runtime().block_on(get_model_info(
        client,
        settings,
        model,
        deployment,
        REQUEST_TIMEOUT,
    ))
}

fn json(source: &str) -> Json {
    serde_json::from_str(source).unwrap()
}

fn text(source: &str) -> String {
    serde_json::to_string(&json(source)).unwrap()
}

fn fields(source: &str) -> IndexMap<String, Json> {
    let Json::Object(fields) = json(source) else {
        panic!("the fixture is not a JSON object");
    };
    fields
}

fn configured_origin_auth() {
    default_origin();
    explicit_origin();
    empty_base_suppresses_ambient_auth();
    ambient_key_precedence();
    auth_headers_by_key();
}

fn default_origin() {
    let origin = resolve_origin(
        &credentials(None, None, None),
        DeploymentCredentials::default(),
    );
    assert_eq!(origin.api_base, DEFAULT_API_BASE);
    assert_eq!(origin.api_key.expose_secret(), DEFAULT_API_KEY);
    assert!(auth_header(&origin.api_key).is_none());
}

fn explicit_origin() {
    let settings = credentials(
        Some("http://env:8000/api/v1"),
        Some("env-key"),
        Some("global-key"),
    );
    let deployment = DeploymentCredentials {
        api_base: Some("http://explicit:9000/api/v1/"),
        api_key: None,
    };
    let origin = resolve_origin(&settings, deployment);
    assert_eq!(origin.api_base, "http://explicit:9000/api/v1/");
    assert_eq!(origin.api_key.expose_secret(), DEFAULT_API_KEY);
    assert!(auth_header(&origin.api_key).is_none());
    let explicit_key = SecretString::from("explicit-key");
    let keyed = resolve_origin(
        &settings,
        DeploymentCredentials {
            api_base: Some("http://explicit:9000/api/v1/"),
            api_key: Some(&explicit_key),
        },
    );
    assert_eq!(keyed.api_key.expose_secret(), "explicit-key");
}

fn empty_base_suppresses_ambient_auth() {
    let settings = credentials(
        Some("http://env:8000/api/v1"),
        Some("env-key"),
        Some("global-key"),
    );
    let origin = resolve_origin(
        &settings,
        DeploymentCredentials {
            api_base: Some(""),
            api_key: None,
        },
    );
    assert_eq!(origin.api_base, "http://env:8000/api/v1");
    assert_eq!(origin.api_key.expose_secret(), DEFAULT_API_KEY);
    assert!(auth_header(&origin.api_key).is_none());
    let empty_key = SecretString::from("");
    let suppressed = resolve_origin(
        &settings,
        DeploymentCredentials {
            api_base: Some(""),
            api_key: Some(&empty_key),
        },
    );
    assert_eq!(suppressed.api_key.expose_secret(), DEFAULT_API_KEY);
}

fn ambient_key_precedence() {
    let settings = credentials(None, Some("env-key"), Some("global-key"));
    let ambient = resolve_origin(&settings, DeploymentCredentials::default());
    assert_eq!(ambient.api_base, DEFAULT_API_BASE);
    assert_eq!(ambient.api_key.expose_secret(), "global-key");
    let env_only = credentials(None, Some("env-key"), None);
    let from_env = resolve_origin(&env_only, DeploymentCredentials::default());
    assert_eq!(from_env.api_key.expose_secret(), "env-key");
    let explicit_key = SecretString::from("explicit-key");
    let explicit = resolve_origin(
        &settings,
        DeploymentCredentials {
            api_base: None,
            api_key: Some(&explicit_key),
        },
    );
    assert_eq!(explicit.api_key.expose_secret(), "explicit-key");
    let empty_key = SecretString::from("");
    let empty = resolve_origin(
        &settings,
        DeploymentCredentials {
            api_base: None,
            api_key: Some(&empty_key),
        },
    );
    assert_eq!(empty.api_key.expose_secret(), "global-key");
    let empty_global = credentials(None, Some("env-key"), Some(""));
    let env_fallback = resolve_origin(&empty_global, DeploymentCredentials::default());
    assert_eq!(env_fallback.api_key.expose_secret(), "env-key");
    let empty_env_key = credentials(None, Some(""), None);
    assert_eq!(
        resolve_origin(&empty_env_key, DeploymentCredentials::default())
            .api_key
            .expose_secret(),
        DEFAULT_API_KEY
    );
    let empty_env_base = credentials(Some(""), None, None);
    assert_eq!(
        resolve_origin(&empty_env_base, DeploymentCredentials::default()).api_base,
        DEFAULT_API_BASE
    );
}

fn auth_headers_by_key() {
    assert!(auth_header(&SecretString::from(DEFAULT_API_KEY)).is_none());
    let header = auth_header(&SecretString::from("other")).unwrap();
    assert_eq!(header.0, "Authorization");
    assert_eq!(header.1.expose_secret(), "Bearer other");
}

fn positive_token_limits() {
    positive_int_values();
    context_window_precedence();
}

fn positive_int_values() {
    let values: &[(&str, Option<u64>)] = &[
        ("1", Some(1)),
        ("7", Some(7)),
        ("0", None),
        ("18446744073709551616", None),
        ("-1", None),
        ("3.0", None),
        ("true", None),
        ("false", None),
        ("null", None),
        ("[]", None),
        ("{}", None),
        (r#""1""#, Some(1)),
        (r#""7""#, Some(7)),
        (r#""  +42  ""#, Some(42)),
        (r#""18446744073709551616""#, None),
        (r#""007""#, Some(7)),
        (r#""1_000""#, Some(1000)),
        (r#""-5""#, None),
        (r#""0""#, None),
        (r#""abc""#, None),
        (r#""""#, None),
        (r#""300.0""#, None),
        (r#""1_""#, None),
        (r#""_1""#, None),
        ("\"\u{664}\u{662}\"", None),
    ];
    for (source, expected) in values {
        assert_eq!(positive_int(&json(source)), *expected, "{source}");
    }
}

fn context_window_precedence() {
    let cases: &[(&str, Option<u64>)] = &[
        (
            r#"{"recipe_options":{"ctx_size":10},"max_input_tokens":20,"context_window":30,"max_context_window":40}"#,
            Some(10),
        ),
        (
            r#"{"max_input_tokens":20,"context_window":30,"max_context_window":40}"#,
            Some(20),
        ),
        (
            r#"{"max_input_tokens":"bad","context_window":30,"max_context_window":40}"#,
            Some(30),
        ),
        (
            r#"{"max_input_tokens":0,"context_window":"","max_context_window":40}"#,
            Some(40),
        ),
        (
            r#"{"max_input_tokens":true,"context_window":1.5,"max_context_window":"x"}"#,
            None,
        ),
        (
            r#"{"recipe_options":{"ctx_size":5.5},"max_input_tokens":9}"#,
            Some(9),
        ),
        (
            r#"{"recipe_options":"not a dict","context_window":3}"#,
            Some(3),
        ),
        (
            r#"{"context_window":null,"max_context_window":"8"}"#,
            Some(8),
        ),
        ("{}", None),
    ];
    for (source, expected) in cases {
        assert_eq!(context_window(&fields(source)), *expected, "{source}");
    }
}

fn provider_metadata_overlay() {
    overlay_by_key_presence();
    overlay_starts_from_a_copy();
    overlay_ignores_non_dict_entries();
}

fn overlay_by_key_presence() {
    let model_info = fields(
        r#"{"provider_specific_entry":{"kept":true,"context_window":1},"recipe_options":{"ctx_size":4},"context_window":null}"#,
    );
    let before = model_info.clone();
    let entry = provider_specific_entry(&model_info);
    assert_eq!(model_info, before);
    assert_eq!(
        serde_json::to_string(&entry).unwrap(),
        r#"{"kept":true,"context_window":null,"recipe_options":{"ctx_size":4}}"#
    );
}

fn overlay_starts_from_a_copy() {
    let model_info = fields(
        r#"{"provider_specific_entry":{"recipe_options":{"ctx_size":1}},"context_window":2,"max_context_window":3}"#,
    );
    assert_eq!(
        serde_json::to_string(&provider_specific_entry(&model_info)).unwrap(),
        r#"{"recipe_options":{"ctx_size":1},"context_window":2,"max_context_window":3}"#
    );
}

fn overlay_ignores_non_dict_entries() {
    let text_entry = fields(r#"{"max_context_window":64,"provider_specific_entry":"text"}"#);
    assert_eq!(
        serde_json::to_string(&provider_specific_entry(&text_entry)).unwrap(),
        r#"{"max_context_window":64}"#
    );
    let empty = fields(r#"{"provider_specific_entry":{}}"#);
    assert!(provider_specific_entry(&empty).is_empty());
    let null_entry = fields(r#"{"provider_specific_entry":null}"#);
    assert!(provider_specific_entry(&null_entry).is_empty());
}

fn model_query_and_defaults() {
    transport_failure_returns_defaults();
    status_failures_return_defaults();
    invalid_bodies_return_defaults();
    structural_malformed_reply_is_an_error();
    model_prefix_and_encoding();
    empty_base_keeps_env_base_without_auth();
    request_carries_the_configured_deadline();
}

fn request_carries_the_configured_deadline() {
    let client = Fixture::answering(200, "{}");
    let timeout = Duration::from_secs(37);
    let info = runtime()
        .block_on(get_model_info(
            &client,
            &credentials(None, None, None),
            "model",
            DeploymentCredentials::default(),
            timeout,
        ))
        .unwrap();
    assert_eq!(info.key, "lemonade/model");
    assert_eq!(client.requests()[0].timeout, timeout);
}

fn transport_failure_returns_defaults() {
    let settings = credentials(Some("http://env:8000/api/v1"), Some("env-key"), None);
    let client = Fixture::failing();
    let info = lookup(
        &client,
        &settings,
        "lemonade/org/model",
        DeploymentCredentials::default(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&info).unwrap(),
        text(
            r#"{"key":"lemonade/org/model","litellm_provider":"lemonade","mode":"chat","input_cost_per_token":0.0,"output_cost_per_token":0.0,"max_tokens":null,"max_input_tokens":null,"max_output_tokens":null}"#
        )
    );
    let requests = client.requests();
    assert_eq!(requests[0].url, "http://env:8000/api/v1/models/org%2Fmodel");
    assert_eq!(requests[0].headers.len(), 1);
    assert_eq!(requests[0].headers[0].0, "Authorization");
    assert_eq!(requests[0].headers[0].1.expose_secret(), "Bearer env-key");
    assert_eq!(requests[0].timeout, REQUEST_TIMEOUT);
}

fn status_failures_return_defaults() {
    let settings = credentials(None, None, None);
    for status in [201, 302, 404, 500] {
        let client = Fixture::answering(status, r#"{"max_input_tokens":8}"#);
        let info = lookup(
            &client,
            &settings,
            "model",
            DeploymentCredentials::default(),
        )
        .unwrap();
        assert_eq!(info.max_input_tokens, (status == 201).then_some(8));
        assert_eq!(info.max_output_tokens, None);
    }
}

fn invalid_bodies_return_defaults() {
    let settings = credentials(None, None, None);
    for body in ["", "not json", r#"{"max_input_tokens":12"#] {
        let client = Fixture::answering(200, body);
        let info = lookup(
            &client,
            &settings,
            "model",
            DeploymentCredentials::default(),
        )
        .unwrap();
        assert_eq!(info.max_input_tokens, None);
        assert_eq!(info.key, "lemonade/model");
    }
}

fn structural_malformed_reply_is_an_error() {
    let client = Fixture::answering(200, r#"[{"max_input_tokens":8}]"#);
    let error = lookup(
        &client,
        &credentials(None, None, None),
        "model",
        DeploymentCredentials::default(),
    )
    .unwrap_err();
    assert!(matches!(error, ModelInfoReplyError::NotObject));
}

fn model_prefix_and_encoding() {
    let settings = credentials(None, None, None);
    let literal_key = SecretString::from(DEFAULT_API_KEY);
    let client = Fixture::answering(200, "{}");
    let info = lookup(
        &client,
        &settings,
        "lemonade/lemonade/deep",
        DeploymentCredentials {
            api_base: Some("http://explicit:9000/"),
            api_key: Some(&literal_key),
        },
    )
    .unwrap();
    assert_eq!(info.key, "lemonade/lemonade/deep");
    let requests = client.requests();
    assert_eq!(
        requests[0].url,
        "http://explicit:9000//models/lemonade%2Fdeep"
    );
    assert!(requests[0].headers.is_empty());
}

fn empty_base_keeps_env_base_without_auth() {
    let settings = credentials(Some("http://env:8000/api/v1"), Some("env-key"), None);
    let client = Fixture::answering(200, "{}");
    let info = lookup(
        &client,
        &settings,
        "m\u{f6}del name_b.c-d~e",
        DeploymentCredentials {
            api_base: Some(""),
            api_key: None,
        },
    )
    .unwrap();
    assert_eq!(info.key, "lemonade/m\u{f6}del name_b.c-d~e");
    let requests = client.requests();
    assert_eq!(
        requests[0].url,
        "http://env:8000/api/v1/models/m%C3%B6del%20name_b.c-d~e"
    );
    assert!(requests[0].headers.is_empty());
}

fn model_limit_projection() {
    projects_all_limits();
    falls_back_to_the_output_limit();
    keeps_the_input_limit_out_of_max_tokens();
    rejects_nonpositive_limits();
    keeps_the_provider_entry();
}

fn projects_all_limits() {
    let settings = credentials(None, None, None);
    let client = Fixture::answering(
        200,
        r#"{"max_input_tokens":100,"max_output_tokens":"64","max_tokens":32,"provider_specific_entry":{"kept":"yes"}}"#,
    );
    let info = lookup(
        &client,
        &settings,
        "model",
        DeploymentCredentials::default(),
    )
    .unwrap();
    assert_eq!(
        serde_json::to_string(&info).unwrap(),
        text(
            r#"{"key":"lemonade/model","litellm_provider":"lemonade","mode":"chat","input_cost_per_token":0.0,"output_cost_per_token":0.0,"max_tokens":32,"max_input_tokens":100,"max_output_tokens":64,"provider_specific_entry":{"kept":"yes"}}"#
        )
    );
    assert_eq!(info.input_cost_per_token.to_string(), "0.0");
    assert_eq!(info.output_cost_per_token.to_string(), "0.0");
}

fn falls_back_to_the_output_limit() {
    let settings = credentials(None, None, None);
    let client = Fixture::answering(200, r#"{"max_input_tokens":100,"max_output_tokens":64}"#);
    let info = lookup(
        &client,
        &settings,
        "model",
        DeploymentCredentials::default(),
    )
    .unwrap();
    assert_eq!(info.max_tokens, Some(64));
    assert_eq!(info.max_input_tokens, Some(100));
    assert!(info.provider_specific_entry.is_none());
}

fn keeps_the_input_limit_out_of_max_tokens() {
    let settings = credentials(None, None, None);
    let client = Fixture::answering(200, r#"{"max_input_tokens":100}"#);
    let info = lookup(
        &client,
        &settings,
        "model",
        DeploymentCredentials::default(),
    )
    .unwrap();
    assert_eq!(info.max_tokens, None);
    assert_eq!(info.max_input_tokens, Some(100));
    assert_eq!(info.max_output_tokens, None);
}

fn rejects_nonpositive_limits() {
    let settings = credentials(None, None, None);
    let client = Fixture::answering(200, r#"{"max_tokens":0,"max_output_tokens":"-1"}"#);
    let info = lookup(
        &client,
        &settings,
        "model",
        DeploymentCredentials::default(),
    )
    .unwrap();
    assert_eq!(info.max_tokens, None);
    assert_eq!(info.max_output_tokens, None);
}

fn keeps_the_provider_entry() {
    let settings = credentials(None, None, None);
    let client = Fixture::answering(
        200,
        r#"{"recipe_options":{"ctx_size":"512"},"max_input_tokens":999,"context_window":2048}"#,
    );
    let info = lookup(
        &client,
        &settings,
        "model",
        DeploymentCredentials::default(),
    )
    .unwrap();
    assert_eq!(info.max_input_tokens, Some(512));
    assert_eq!(
        serde_json::to_string(&info.provider_specific_entry).unwrap(),
        r#"{"recipe_options":{"ctx_size":"512"},"context_window":2048}"#
    );
}

#[test]
fn lemonade_model_metadata_credential_identity_isolation() {
    let first = credentials(
        Some("http://first:8000/api/v1"),
        Some("first-key"),
        Some("first-global"),
    );
    let second = credentials(
        Some("http://second:8000/api/v1"),
        Some("second-key"),
        Some("second-global"),
    );
    let first_origin = resolve_origin(&first, DeploymentCredentials::default());
    let second_origin = resolve_origin(&second, DeploymentCredentials::default());
    assert_ne!(first_origin.api_base, second_origin.api_base);
    assert_ne!(
        first_origin.api_key.expose_secret(),
        second_origin.api_key.expose_secret()
    );
    let first_header = auth_header(&first_origin.api_key).unwrap();
    let second_header = auth_header(&second_origin.api_key).unwrap();
    assert_eq!(first_header.0, "Authorization");
    assert_ne!(
        first_header.1.expose_secret(),
        second_header.1.expose_secret()
    );
    let repeated = resolve_origin(&first, DeploymentCredentials::default());
    assert_eq!(repeated.api_base, first_origin.api_base);
    assert_eq!(
        repeated.api_key.expose_secret(),
        first_origin.api_key.expose_secret()
    );
}

#[test]
fn lemonade_model_metadata_secret_safe_failure() {
    let settings = credentials(
        Some("http://fixture:8000/api/v1"),
        Some("fixture-secret-key"),
        Some("fixture-global-key"),
    );
    let client = Fixture::failing();
    let info = lookup(
        &client,
        &settings,
        "model",
        DeploymentCredentials::default(),
    )
    .unwrap();
    let rendered = serde_json::to_string(&info).unwrap();
    assert!(!rendered.contains("fixture-secret-key"));
    assert!(!rendered.contains("fixture-global-key"));
    assert!(!format!("{settings:?}").contains("fixture-secret-key"));
    assert!(!format!("{settings:?}").contains("fixture-global-key"));
    let origin = resolve_origin(&settings, DeploymentCredentials::default());
    assert!(!format!("{origin:?}").contains("fixture-global-key"));
    let requests = client.requests();
    let request = requests.first().unwrap();
    assert!(!format!("{request:?}").contains("fixture-secret-key"));
    assert!(!format!("{request:?}").contains("fixture-global-key"));
}

#[rstest]
fn replays_the_lemonade_model_reply_corpus() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/lemonade_model_reply");
    let mut seen = 0_usize;
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        let bytes = fs::read(&path).unwrap();
        let category = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default()
            .to_owned();
        replay_one(&category, &bytes);
        seen = seen.saturating_add(1);
    }
    assert!(seen > 0);
}

fn replay_one(category: &str, bytes: &[u8]) {
    let outcome = decode_lemonade_model_reply(bytes);
    match category.split('-').next().unwrap_or_default() {
        "valid" | "boundary" => assert!(outcome.is_ok(), "{category}"),
        "not" => assert!(
            matches!(outcome, Err(ModelInfoReplyError::NotObject)),
            "{category}"
        ),
        "malformed" | "truncated" | "empty" => assert!(
            matches!(outcome, Err(ModelInfoReplyError::Json { .. })),
            "{category}"
        ),
        _ => drop(outcome),
    }
}
