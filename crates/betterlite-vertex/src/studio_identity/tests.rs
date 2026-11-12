#![cfg(test)]

use std::collections::BTreeMap;
use std::future::{Future, ready};
use std::path::Path;
use std::sync::Mutex;
use std::time::Duration;
use std::{fs, io};

use betterlite_config::sections::studio_identity::Settings;
use betterlite_http::proxy::UpstreamResponse;
use betterlite_http::proxy::outbound::OutboundRequest;
use betterlite_http::proxy::transport_error::TransportError;
use http::Method;
use http::header::HeaderMap;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::decode::{MODEL_LIST_LIMIT, decode_studio_models};
use super::error::StudioIdentityError;
use super::models::{ModelsTransport, get_models, models_request};
use super::{
    AuthValue, Headers, NativeParams, get_api_base, get_api_key, native_api_key, native_headers,
    process_model_names,
};

const DEADLINE: Duration = Duration::from_secs(5);

const DEFAULT_BASE: &str = "https://generativelanguage.googleapis.com";

fn secret(text: &str) -> SecretString {
    SecretString::from(text)
}

fn settings(environment: Option<&str>, generic: Option<&str>, base: Option<&str>) -> Settings {
    Settings {
        environment_key: environment.map(secret),
        generic_api_key: generic.map(secret),
        api_base: base.map(str::to_owned),
    }
}

fn headers(pairs: &[(&str, &str)]) -> Headers {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

fn secret_headers(pairs: &[(&str, &str)]) -> BTreeMap<String, SecretString> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), secret(value)))
        .collect()
}

fn auth_text(auth: Option<AuthValue>) -> Option<String> {
    match auth {
        Some(AuthValue::Key(key)) => Some(key.expose_secret().to_owned()),
        Some(AuthValue::Headers(map)) => map
            .get("Authorization")
            .map(|value| value.expose_secret().to_owned()),
        None => None,
    }
}

#[rstest]
#[case::explicit_wins(Some("explicit-key"), Some("environment-key"), Some("explicit-key"))]
#[case::environment_when_absent(None, Some("environment-key"), Some("environment-key"))]
#[case::nothing_anywhere(None, None, None)]
fn projection_api_key_prefers_explicit_then_environment(
    #[case] explicit: Option<&str>,
    #[case] environment: Option<&str>,
    #[case] expected: Option<&str>,
) {
    let settings = settings(environment, Some("generic-key"), None);
    let found = get_api_key(explicit.map(secret).as_ref(), &settings);
    assert_eq!(found.as_ref().map(ExposeSecret::expose_secret), expected);
}

#[test]
fn projection_api_key_uses_an_explicit_empty_key_as_given() {
    let found = get_api_key(Some(&secret("")), &Settings::default());
    assert_eq!(found.as_ref().map(ExposeSecret::expose_secret), Some(""));
}

#[test]
fn projection_api_key_skips_the_generic_litellm_key() {
    let found = get_api_key(None, &settings(None, Some("generic-key"), None));
    assert!(found.is_none());
}

#[rstest]
#[case::explicit_wins(
    (Some("https://explicit.example"), Some("https://environment.example")),
    "https://explicit.example"
)]
#[case::environment_when_absent(
    (None, Some("https://environment.example")),
    "https://environment.example"
)]
#[case::default_when_nothing_is_set((None, None), DEFAULT_BASE)]
fn projection_api_base_prefers_explicit_then_environment_then_default(
    #[case] base: (Option<&str>, Option<&str>),
    #[case] expected: &str,
) {
    let (explicit, environment) = base;
    assert_eq!(
        get_api_base(explicit, &settings(None, None, environment)),
        expected
    );
}

#[test]
fn projection_api_base_uses_an_explicit_empty_base_as_given() {
    let settings = settings(None, None, Some("https://environment.example"));
    assert_eq!(get_api_base(Some(""), &settings), "");
}

#[rstest]
#[case::explicit(
    Some(AuthValue::Key(secret("explicit-key"))),
    Some(AuthValue::Key(secret("params-key"))),
    Some(AuthValue::Key(secret("params-gemini-key"))),
    Some("explicit-key")
)]
#[case::params_api_key(
    None,
    Some(AuthValue::Key(secret("params-key"))),
    Some(AuthValue::Key(secret("params-gemini-key"))),
    Some("params-key")
)]
#[case::params_gemini_key(
    None,
    None,
    Some(AuthValue::Key(secret("params-gemini-key"))),
    Some("params-gemini-key")
)]
#[case::environment(None, None, None, Some("environment-key"))]
#[case::generic(None, None, None, Some("generic-key"))]
#[case::nothing(None, None, None, None)]
fn projection_native_api_key_follows_the_litellm_chain(
    #[case] explicit: Option<AuthValue>,
    #[case] params_api_key: Option<AuthValue>,
    #[case] params_gemini_key: Option<AuthValue>,
    #[case] expected: Option<&str>,
) {
    let params = NativeParams {
        api_key: params_api_key,
        gemini_api_key: params_gemini_key,
    };
    let settings = settings(Some("environment-key"), Some("generic-key"), None);
    let found = native_api_key(explicit.as_ref(), &params, &settings);
    assert_eq!(auth_text(found).as_deref(), expected);
}

#[test]
fn projection_native_api_key_keeps_an_auth_map_from_the_params() {
    let params = NativeParams {
        api_key: Some(AuthValue::Headers(secret_headers(&[(
            "Authorization",
            "Bearer mapped",
        )]))),
        gemini_api_key: None,
    };
    let found = native_api_key(None, &params, &Settings::default());
    assert_eq!(auth_text(found), Some("Bearer mapped".to_owned()));
}

#[test]
fn projection_native_api_key_leaves_the_deployment_params_intact() {
    let params = NativeParams {
        api_key: Some(AuthValue::Key(secret("deployment-key"))),
        gemini_api_key: Some(AuthValue::Key(secret("deployment-gemini"))),
    };
    let first = native_api_key(None, &params, &Settings::default());
    let second = native_api_key(None, &params, &Settings::default());
    assert_eq!(auth_text(first), Some("deployment-key".to_owned()));
    assert_eq!(auth_text(second), Some("deployment-key".to_owned()));
}

#[test]
fn projection_native_headers_start_with_a_json_content_type() {
    let result = native_headers(
        None,
        &NativeParams::default(),
        &Settings::default(),
        Headers::new(),
    );
    assert_eq!(result, headers(&[("Content-Type", "application/json")]));
}

#[test]
fn projection_native_headers_add_the_resolved_key() {
    let settings = settings(Some("environment-key"), None, None);
    let result = native_headers(None, &NativeParams::default(), &settings, Headers::new());
    assert_eq!(
        result,
        headers(&[
            ("Content-Type", "application/json"),
            ("x-goog-api-key", "environment-key")
        ])
    );
}

#[test]
fn projection_native_headers_merge_an_auth_map() {
    let explicit = AuthValue::Headers(secret_headers(&[
        ("Authorization", "Bearer mapped"),
        ("X-Goog-User-Project", "project"),
    ]));
    let result = native_headers(
        Some(&explicit),
        &NativeParams::default(),
        &Settings::default(),
        Headers::new(),
    );
    assert_eq!(
        result,
        headers(&[
            ("Content-Type", "application/json"),
            ("Authorization", "Bearer mapped"),
            ("X-Goog-User-Project", "project")
        ])
    );
}

#[test]
fn projection_native_headers_let_caller_headers_win() {
    let settings = settings(Some("environment-key"), None, None);
    let caller = headers(&[
        ("Content-Type", "text/plain"),
        ("x-goog-api-key", "caller-key"),
        ("X-Caller", "kept"),
    ]);
    let result = native_headers(None, &NativeParams::default(), &settings, caller);
    assert_eq!(
        result,
        headers(&[
            ("Content-Type", "text/plain"),
            ("x-goog-api-key", "caller-key"),
            ("X-Caller", "kept")
        ])
    );
}

#[test]
fn projection_native_headers_keep_other_casings_side_by_side() {
    let caller = headers(&[("CONTENT-TYPE", "text/plain")]);
    let result = native_headers(None, &NativeParams::default(), &Settings::default(), caller);
    assert_eq!(
        result,
        headers(&[
            ("CONTENT-TYPE", "text/plain"),
            ("Content-Type", "application/json")
        ])
    );
}

#[test]
fn projection_model_names_replace_every_models_prefix() {
    let names = vec![
        "models/gemini-2.0-flash".to_owned(),
        "models/models/gemini-2.0-pro".to_owned(),
        "gemini-1.5-pro".to_owned(),
        "models/".to_owned(),
    ];
    assert_eq!(
        process_model_names(&names),
        vec![
            "gemini/gemini-2.0-flash".to_owned(),
            "gemini/gemini-2.0-pro".to_owned(),
            "gemini/gemini-1.5-pro".to_owned(),
            "gemini/".to_owned(),
        ]
    );
    assert!(process_model_names(&[]).is_empty());
}

#[derive(Clone)]
enum Reply {
    Response { status: u16, body: Vec<u8> },
    Failure,
}

#[derive(Clone)]
struct Recorded {
    method: Method,
    url: String,
    headers: HeaderMap,
}

struct Script {
    reply: Reply,
    requests: Mutex<Vec<Recorded>>,
}

impl Script {
    fn replying(status: u16, body: &[u8]) -> Self {
        Self {
            reply: Reply::Response {
                status,
                body: body.to_vec(),
            },
            requests: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        Self {
            reply: Reply::Failure,
            requests: Mutex::new(Vec::new()),
        }
    }

    fn requests(&self) -> Vec<Recorded> {
        self.requests.lock().unwrap().clone()
    }
}

impl ModelsTransport for Script {
    fn send(
        &self,
        request: OutboundRequest,
    ) -> impl Future<Output = Result<UpstreamResponse, TransportError>> + Send {
        let method = request.method;
        let url = request.url.to_string();
        let headers = request.headers;
        self.requests.lock().unwrap().push(Recorded {
            method,
            url,
            headers,
        });
        let reply = self.reply.clone();
        ready(match reply {
            Reply::Response { status, body } => Ok(UpstreamResponse {
                status,
                headers: HeaderMap::new(),
                body,
            }),
            Reply::Failure => Err(TransportError::Closed),
        })
    }
}

#[tokio::test]
async fn model_list_http_fetches_one_get_in_returned_order() {
    let script = Script::replying(
        200,
        br#"{"models":[{"name":"models/gemini-2.0-flash"},{"name":"models/gemini-1.5-pro"}]}"#,
    );
    let settings = settings(Some("environment-key"), None, None);
    let models = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .expect("the list decodes");
    assert_eq!(
        models,
        vec![
            "gemini/gemini-2.0-flash".to_owned(),
            "gemini/gemini-1.5-pro".to_owned()
        ]
    );
    let sent = script.requests();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].method, Method::GET);
    assert_eq!(sent[0].url, format!("{DEFAULT_BASE}/v1beta/models"));
    let key = sent[0]
        .headers
        .get("x-goog-api-key")
        .expect("the key header");
    assert_eq!(key.to_str().expect("visible header"), "environment-key");
    assert!(key.is_sensitive());
}

#[tokio::test]
async fn model_list_http_prefers_the_explicit_base_and_key() {
    let script = Script::replying(200, br#"{"models":[]}"#);
    let settings = settings(
        Some("environment-key"),
        None,
        Some("https://environment.example"),
    );
    let models = get_models(
        &script,
        Some(&secret("explicit-key")),
        Some("https://explicit.example/"),
        &settings,
        DEADLINE,
    )
    .await
    .expect("the list decodes");
    assert!(models.is_empty());
    let sent = script.requests();
    assert_eq!(sent[0].url, "https://explicit.example//v1beta/models");
    assert_eq!(
        sent[0].headers["x-goog-api-key"].to_str().expect("visible"),
        "explicit-key"
    );
}

#[tokio::test]
async fn model_list_http_requires_a_key() {
    let script = Script::replying(200, br#"{"models":[]}"#);
    let error = get_models(&script, None, None, &Settings::default(), DEADLINE)
        .await
        .expect_err("a key is required");
    assert!(matches!(error, StudioIdentityError::MissingCredentials));
    assert!(script.requests().is_empty());
}

#[tokio::test]
async fn model_list_http_ignores_the_generic_litellm_key() {
    let script = Script::replying(200, br#"{"models":[]}"#);
    let settings = settings(None, Some("generic-key"), None);
    let error = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .expect_err("the generic key is not a Studio key");
    assert!(matches!(error, StudioIdentityError::MissingCredentials));
    assert!(script.requests().is_empty());
}

#[rstest]
#[case::server_error(500)]
#[case::created(201)]
#[case::no_content(204)]
#[case::redirect(302)]
#[tokio::test]
async fn model_list_http_preserves_a_non_200_status(#[case] status: u16) {
    let script = Script::replying(status, b"temporarily unavailable");
    let settings = settings(Some("environment-key"), None, None);
    let error = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .expect_err("only 200 is a model list");
    match error {
        StudioIdentityError::Status {
            status: found,
            body,
        } => {
            assert_eq!(found, status);
            assert_eq!(body, "temporarily unavailable");
        }
        other @ (StudioIdentityError::MissingCredentials
        | StudioIdentityError::ResponseTooLarge { .. }
        | StudioIdentityError::Decode(_)
        | StudioIdentityError::InvalidUrl { .. }
        | StudioIdentityError::InvalidHeader { .. }
        | StudioIdentityError::Transport { .. }) => panic!("unexpected error: {other:?}"),
    }
    assert_eq!(script.requests().len(), 1);
}

#[tokio::test]
async fn model_list_http_does_not_follow_next_page_tokens() {
    let script = Script::replying(
        200,
        br#"{"models":[{"name":"models/gemini-2.0-flash"}],"nextPageToken":"page-2"}"#,
    );
    let settings = settings(Some("environment-key"), None, None);
    let models = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .expect("the list decodes");
    assert_eq!(models, vec!["gemini/gemini-2.0-flash".to_owned()]);
    assert_eq!(script.requests().len(), 1);
}

#[tokio::test]
async fn model_list_http_reports_a_transport_failure() {
    let script = Script::failing();
    let settings = settings(Some("environment-key"), None, None);
    let error = get_models(&script, None, None, &settings, DEADLINE)
        .await
        .expect_err("the transport closed");
    assert!(matches!(error, StudioIdentityError::Transport { .. }));
}

#[test]
fn model_list_http_decodes_only_model_names() {
    let decoded = decode_studio_models(
        200,
        br#"{"models":[{"name":"models/gemini-2.0-flash","displayName":"Flash","supportedGenerationMethods":["generateContent"]}]}"#,
    )
    .expect("the list decodes");
    assert_eq!(decoded, vec!["gemini/gemini-2.0-flash".to_owned()]);
    assert!(matches!(
        decode_studio_models(200, br#"{"models":[{"displayName":"no name"}]}"#),
        Err(StudioIdentityError::Decode(_))
    ));
    assert!(matches!(
        decode_studio_models(200, br#"{"models":[{"name":7}]}"#),
        Err(StudioIdentityError::Decode(_))
    ));
    assert!(matches!(
        decode_studio_models(200, br#"{"data":[]}"#),
        Err(StudioIdentityError::Decode(_))
    ));
    assert!(matches!(
        decode_studio_models(200, b"not json"),
        Err(StudioIdentityError::Decode(_))
    ));
}

#[test]
fn model_list_http_boundary_is_four_mibibytes() {
    assert_eq!(MODEL_LIST_LIMIT, 4 * 1024 * 1024);
    let mut at_limit = br#"{"models":[]}"#.to_vec();
    at_limit.resize(MODEL_LIST_LIMIT, b' ');
    decode_studio_models(200, &at_limit).expect("the list fits the limit");
    let mut over_limit = br#"{"models":[]}"#.to_vec();
    over_limit.resize(MODEL_LIST_LIMIT + 1, b' ');
    assert!(matches!(
        decode_studio_models(200, &over_limit),
        Err(StudioIdentityError::ResponseTooLarge {
            limit: MODEL_LIST_LIMIT
        })
    ));
}

#[test]
fn model_list_http_refuses_a_key_a_header_cannot_carry() {
    let request = models_request(Some(&secret("bad\nkey")), None, &Settings::default())
        .expect("a request is built");
    assert!(matches!(
        request.into_outbound(DEADLINE),
        Err(StudioIdentityError::InvalidHeader {
            name: "x-goog-api-key",
            ..
        })
    ));
}

#[test]
fn model_list_http_refuses_an_invalid_base() {
    let request = models_request(
        Some(&secret("key")),
        Some("not a url"),
        &Settings::default(),
    )
    .expect("a request is built");
    assert!(matches!(
        request.into_outbound(DEADLINE),
        Err(StudioIdentityError::InvalidUrl { .. })
    ));
}

#[test]
fn replays_the_studio_models_corpus() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/studio_models");
    fs::read_dir(corpus)
        .expect("corpus folder")
        .try_for_each(|entry| {
            let path = entry?.path();
            let bytes = fs::read(&path)?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("utf-8 file name");
            let decoded = decode_studio_models(200, &bytes);
            if name.starts_with("valid") {
                decoded.expect("this seed is a model list");
            } else {
                decoded.expect_err("this seed is not a model list");
            }
            Ok::<(), io::Error>(())
        })
        .expect("readable corpus");
}
