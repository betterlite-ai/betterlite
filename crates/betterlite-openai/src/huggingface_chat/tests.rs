#![cfg(test)]

use std::collections::VecDeque;
use std::fs::{read, read_dir, read_to_string};
use std::iter::repeat_n;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use betterlite_http::proxy::transport_error::TransportError;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::{Value, from_str, json};
use tracing::subscriber::with_default;
use tracing_subscriber::fmt;

use super::HuggingfaceEnv;
use super::error::HuggingfaceChatError;
use super::headers::validate_environment;
use super::mapping::{
    HubGet, HubResponse, ProviderMapping, ProviderMappingCache, ProviderMappingSource,
    decode_provider_mapping,
};
use super::request::transform_request;
use super::url::{build_chat_completion_url, get_complete_url};
use crate::chat::ChatMessage;
use crate::openai_base_config::{Headers, Object};

fn env(hf_api_base: Option<&str>, huggingface_api_base: Option<&str>) -> HuggingfaceEnv {
    HuggingfaceEnv {
        hf_api_base: hf_api_base.map(str::to_owned),
        huggingface_api_base: huggingface_api_base.map(str::to_owned),
    }
}

fn chat_messages(source: &str) -> Vec<ChatMessage> {
    from_str(source).unwrap()
}

fn optional_params(source: &str) -> Object {
    from_str(source).unwrap()
}

fn value(source: &str) -> Value {
    from_str(source).unwrap()
}

fn pointer(body: &Object, path: &str) -> Value {
    Value::Object(body.clone())
        .pointer(path)
        .cloned()
        .unwrap_or(Value::Null)
}

fn capture_warnings<T, F>(run: F) -> (T, String)
where
    F: FnOnce() -> T,
{
    let log = tempfile::NamedTempFile::new().unwrap();
    let subscriber = fmt()
        .without_time()
        .with_ansi(false)
        .with_writer(Arc::new(log.reopen().unwrap()))
        .finish();
    let result = with_default(subscriber, run);
    (result, read_to_string(log.path()).unwrap())
}

#[rstest]
#[case::v1_path("https://example.com/v1/", "https://example.com/v1/chat/completions")]
#[case::bare_host("https://example.com", "https://example.com/v1/chat/completions")]
#[case::complete(
    "https://example.com/v1/chat/completions//",
    "https://example.com/v1/chat/completions"
)]
#[case::trailing_slashes("https://example.com///", "https://example.com/v1/chat/completions")]
fn url_api_base(#[case] api_base: &str, #[case] expected: &str) {
    let url = get_complete_url(Some(api_base), "org/model", &HuggingfaceEnv::default()).unwrap();
    assert_eq!(url, expected);
}

#[rstest]
#[case::hf_api_base(
    Some("https://hf.example/v1"),
    Some("https://other.example"),
    "https://hf.example/v1"
)]
#[case::only_huggingface_api_base(None, Some("https://other.example"), "None")]
#[case::empty_hf_api_base(Some(""), Some("https://other.example"), "https://other.example")]
#[case::env_value_keeps_no_trailing_slash(
    Some("https://hf.example/v1/"),
    None,
    "https://hf.example/v1"
)]
#[case::empty_huggingface_api_base(
    None,
    Some(""),
    "https://router.huggingface.co/v1/chat/completions"
)]
fn url_env_branches(
    #[case] hf_api_base: Option<&str>,
    #[case] huggingface_api_base: Option<&str>,
    #[case] expected: &str,
) {
    let url = get_complete_url(None, "org/model", &env(hf_api_base, huggingface_api_base)).unwrap();
    assert_eq!(url, expected);
}

#[rstest]
#[case::hf_inference(
    "hf-inference/org/model",
    "https://router.huggingface.co/hf-inference/models/hf-inference/org/model/v1/chat/completions"
)]
#[case::novita(
    "novita/org/model",
    "https://router.huggingface.co/novita/v3/openai/chat/completions"
)]
#[case::fireworks(
    "fireworks-ai/org/model",
    "https://router.huggingface.co/fireworks-ai/inference/v1/chat/completions"
)]
#[case::other_provider(
    "together/org/model",
    "https://router.huggingface.co/together/v1/chat/completions"
)]
#[case::one_slash("org/model", "https://router.huggingface.co/v1/chat/completions")]
fn url_router_routes(#[case] model: &str, #[case] expected: &str) {
    let url = get_complete_url(None, model, &HuggingfaceEnv::default()).unwrap();
    assert_eq!(url, expected);
}

#[test]
fn url_api_base_wins_over_the_env_branch() {
    let url = get_complete_url(
        Some("https://api.example"),
        "org/model",
        &env(Some("https://hf.example"), None),
    )
    .unwrap();
    assert_eq!(url, "https://api.example/v1/chat/completions");
}

#[test]
fn url_env_branch_wins_over_a_model_url() {
    let url = get_complete_url(
        None,
        "https://model.example/v1",
        &env(Some("https://hf.example"), None),
    )
    .unwrap();
    assert_eq!(url, "https://hf.example");
}

#[test]
fn url_model_url_is_built_into_a_chat_completion_url() {
    let url =
        get_complete_url(None, "https://model.example/v1", &HuggingfaceEnv::default()).unwrap();
    assert_eq!(url, "https://model.example/v1/chat/completions");
}

#[test]
fn url_model_without_slash_is_a_typed_error() {
    let error = get_complete_url(None, "model", &HuggingfaceEnv::default()).unwrap_err();
    assert!(
        matches!(error, HuggingfaceChatError::MissingProviderPrefix { .. }),
        "{error}"
    );
}

#[test]
fn url_build_chat_completion_url_appends_the_route() {
    assert_eq!(
        build_chat_completion_url("https://example.com/v1"),
        "https://example.com/v1/chat/completions"
    );
}

#[test]
fn headers_default_to_a_json_content_type() {
    let headers = validate_environment(&Headers::new(), None);
    assert_eq!(
        headers.get("content-type"),
        Some(&"application/json".to_owned())
    );
    assert_eq!(headers.get("Authorization"), None);
}

#[test]
fn headers_add_a_bearer_only_for_a_given_key() {
    let key = SecretString::from("sk-hf");
    let headers = validate_environment(&Headers::new(), Some(&key));
    assert_eq!(
        headers.get("Authorization"),
        Some(&"Bearer sk-hf".to_owned())
    );
}

#[test]
fn headers_bear_an_empty_key_too() {
    let key = SecretString::from("");
    let headers = validate_environment(&Headers::new(), Some(&key));
    assert_eq!(headers.get("Authorization"), Some(&"Bearer ".to_owned()));
}

#[test]
fn headers_defaults_win_over_caller_headers() {
    let caller = Headers::from([
        ("Authorization".to_owned(), "Bearer other".to_owned()),
        ("content-type".to_owned(), "text/plain".to_owned()),
        ("x-call".to_owned(), "kept".to_owned()),
    ]);
    let key = SecretString::from("sk-hf");
    let headers = validate_environment(&caller, Some(&key));
    assert_eq!(
        headers.get("Authorization"),
        Some(&"Bearer sk-hf".to_owned())
    );
    assert_eq!(
        headers.get("content-type"),
        Some(&"application/json".to_owned())
    );
    assert_eq!(headers.get("x-call"), Some(&"kept".to_owned()));
}

#[test]
fn headers_get_error_class_carries_status_message_and_headers() {
    let headers = Headers::from([("x-retry".to_owned(), "1".to_owned())]);
    let error = super::error::get_error_class("slow down", 429, &headers);
    assert_eq!(error.status_code, 429);
    assert_eq!(error.message, "slow down");
    assert_eq!(error.headers, headers);
}

struct RefusingSource;

impl ProviderMappingSource for RefusingSource {
    fn mapping(&self, model_id: &str) -> Result<ProviderMapping, HuggingfaceChatError> {
        Err(HuggingfaceChatError::NoProviderMapping {
            model: model_id.to_owned(),
        })
    }
}

struct FixedSource {
    mapping: ProviderMapping,
    asked: Mutex<Vec<String>>,
}

impl ProviderMappingSource for FixedSource {
    fn mapping(&self, model_id: &str) -> Result<ProviderMapping, HuggingfaceChatError> {
        self.asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(model_id.to_owned());
        Ok(self.mapping.clone())
    }
}

fn source(mapping: &str) -> FixedSource {
    FixedSource {
        mapping: from_str(mapping).unwrap(),
        asked: Mutex::new(Vec::new()),
    }
}

#[test]
fn request_api_base_skips_mapping_and_message_transform() {
    let messages = chat_messages(
        r#"[{"role":"user","content":[{"type":"text","text":"hi"},{"type":"image_url","image_url":"https://example.com/a.png"}]}]"#,
    );
    let options = optional_params(r#"{"max_retries":2}"#);
    let body = transform_request(
        "novita/org/model",
        &messages,
        &options,
        Some("http://mock"),
        &RefusingSource,
    )
    .unwrap();
    assert_eq!(body.get("model"), Some(&json!("novita/org/model")));
    assert_eq!(
        pointer(&body, "/messages/0/content/1/image_url"),
        json!("https://example.com/a.png")
    );
    assert_eq!(body.get("max_retries"), Some(&json!(2_i32)));
}

#[test]
fn request_max_retries_warns_and_is_dropped() {
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let options = optional_params(r#"{"max_retries":2,"temperature":0.5}"#);
    let (outcome, warnings) = capture_warnings(|| {
        transform_request("org/model", &messages, &options, None, &RefusingSource)
    });
    let body = outcome.unwrap();
    assert_eq!(body.get("max_retries"), None);
    assert_eq!(body.get("temperature"), Some(&json!(0.5_f64)));
    assert!(
        warnings.contains("`max_retries` is not supported. It will be ignored."),
        "{warnings}"
    );
}

#[test]
fn request_sends_the_mapped_provider_id_and_asks_for_the_model_id() {
    let mapping = source(r#"{"novita": {"status": "live", "providerId": "org/mapped"}}"#);
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "novita/org/model",
        &messages,
        &Object::new(),
        None,
        &mapping,
    )
    .unwrap();
    assert_eq!(body.get("model"), Some(&json!("org/mapped")));
    assert_eq!(*mapping.asked.lock().unwrap(), vec!["org/model".to_owned()]);
}

#[test]
fn request_ignores_an_incomplete_entry_for_another_provider() {
    let mapping = source(
        r#"{"novita": {"status": "live", "providerId": "org/mapped"}, "other": {"status": "live"}}"#,
    );
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "novita/org/model",
        &messages,
        &Object::new(),
        None,
        &mapping,
    )
    .unwrap();
    assert_eq!(body.get("model"), Some(&json!("org/mapped")));
}

#[rstest]
#[case::missing_provider_id(r#"{"novita": {"status": "live"}}"#, "providerId")]
#[case::missing_status(r#"{"novita": {"providerId": "org/mapped"}}"#, "status")]
fn request_an_incomplete_selected_entry_is_a_typed_error(#[case] entry: &str, #[case] field: &str) {
    let mapping = source(entry);
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let error = transform_request(
        "novita/org/model",
        &messages,
        &Object::new(),
        None,
        &mapping,
    )
    .unwrap_err();
    let HuggingfaceChatError::IncompleteProviderMapping { field: named, .. } = &error else {
        panic!("unexpected {error:?}");
    };
    assert_eq!(*named, field);
}

#[rstest]
#[case::tool_calls_mapped(
    r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}"#,
    None
)]
#[case::tool_calls_api_base(
    r#"{"role":"assistant","content":null,"tool_calls":[{"id":"call_1","type":"function","function":{"name":"lookup","arguments":"{}"}}]}"#,
    Some("http://mock")
)]
#[case::function_call_mapped(
    r#"{"role":"assistant","content":null,"function_call":{"name":"lookup","arguments":"{}"}}"#,
    None
)]
#[case::function_call_api_base(
    r#"{"role":"assistant","content":null,"function_call":{"name":"lookup","arguments":"{}"}}"#,
    Some("http://mock")
)]
#[case::function_message_mapped(r#"{"role":"function","name":"lookup","content":null}"#, None)]
#[case::function_message_api_base(
    r#"{"role":"function","name":"lookup","content":null}"#,
    Some("http://mock")
)]
fn request_keeps_a_tool_call_turn_content_null(
    #[case] message: &str,
    #[case] api_base: Option<&str>,
) {
    let mapping = source(r#"{"novita": {"status": "live", "providerId": "org/mapped"}}"#);
    let messages = chat_messages(&format!("[{message}]"));
    let body = transform_request(
        "novita/org/model",
        &messages,
        &Object::new(),
        api_base,
        &mapping,
    )
    .unwrap();
    let turn = body["messages"][0].as_object().unwrap();
    assert_eq!(turn.get("content"), Some(&Value::Null));
}

#[rstest]
#[case::assistant_without_calls(r#"{"role":"assistant","name":"lookup"}"#)]
#[case::user_without_content(r#"{"role":"user"}"#)]
fn request_keeps_absent_content_absent(#[case] message: &str) {
    let messages = chat_messages(&format!("[{message}]"));
    let body = transform_request(
        "org/model",
        &messages,
        &Object::new(),
        None,
        &RefusingSource,
    )
    .unwrap();
    let turn = body["messages"][0].as_object().unwrap();
    assert_eq!(turn.get("content"), None);
}

#[test]
fn request_staging_warns_and_still_sends_the_provider_id() {
    let mapping = source(r#"{"novita": {"status": "staging", "providerId": "org/mapped"}}"#);
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let (outcome, warnings) = capture_warnings(|| {
        transform_request(
            "novita/org/model",
            &messages,
            &Object::new(),
            None,
            &mapping,
        )
    });
    assert_eq!(outcome.unwrap().get("model"), Some(&json!("org/mapped")));
    assert!(
        warnings.contains(
            "Model org/model is in staging mode for provider novita. Meant for test purposes only."
        ),
        "{warnings}"
    );
}

#[test]
fn request_unsupported_provider_is_a_404() {
    let mapping = source(r#"{"other": {"status": "live", "providerId": "org/mapped"}}"#);
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let error = transform_request(
        "novita/org/model",
        &messages,
        &Object::new(),
        None,
        &mapping,
    )
    .unwrap_err();
    let HuggingfaceChatError::Huggingface(failure) = &error else {
        panic!("unexpected {error:?}");
    };
    assert_eq!(failure.status_code, 404);
    assert!(failure.headers.is_empty());
    assert_eq!(
        failure.message,
        "Model org/model is not supported for provider novita"
    );
}

#[test]
fn request_one_slash_sends_the_model_without_asking_for_a_mapping() {
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "org/model",
        &messages,
        &Object::new(),
        None,
        &RefusingSource,
    )
    .unwrap();
    assert_eq!(body.get("model"), Some(&json!("org/model")));
}

#[test]
fn request_model_without_slash_is_a_typed_error() {
    let messages = chat_messages(r#"[{"role":"user","content":"hi"}]"#);
    let error =
        transform_request("model", &messages, &Object::new(), None, &RefusingSource).unwrap_err();
    assert!(
        matches!(error, HuggingfaceChatError::MissingProviderPrefix { .. }),
        "{error}"
    );
}

#[test]
fn request_transforms_a_user_content_image_url_string() {
    let messages = chat_messages(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"https://example.com/a.png"}]}]"#,
    );
    let body = transform_request(
        "org/model",
        &messages,
        &Object::new(),
        None,
        &RefusingSource,
    )
    .unwrap();
    assert_eq!(
        pointer(&body, "/messages/0/content/0/image_url"),
        value(r#"{"url":"https://example.com/a.png"}"#)
    );
}

#[test]
fn request_strips_cache_control_from_forwarded_messages() {
    let messages = chat_messages(
        r#"[{"role":"user","content":[{"type":"text","text":"hi","cache_control":{"type":"ephemeral"}}]}]"#,
    );
    let body = transform_request(
        "org/model",
        &messages,
        &Object::new(),
        None,
        &RefusingSource,
    )
    .unwrap();
    assert_eq!(
        pointer(&body, "/messages/0/content/0/cache_control"),
        Value::Null
    );
}

#[test]
fn request_an_empty_api_base_does_not_take_the_shortcut() {
    let messages = chat_messages(
        r#"[{"role":"user","content":[{"type":"image_url","image_url":"https://example.com/a.png"}]}]"#,
    );
    let body = transform_request(
        "org/model",
        &messages,
        &Object::new(),
        Some(""),
        &RefusingSource,
    )
    .unwrap();
    assert_eq!(
        pointer(&body, "/messages/0/content/0/image_url"),
        value(r#"{"url":"https://example.com/a.png"}"#)
    );
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Call {
    url: String,
    headers: Headers,
    query: Vec<(String, String)>,
}

#[derive(Clone)]
enum Reply {
    Response(HubResponse),
    Failure,
    EchoModel,
}

struct ScriptedHub {
    replies: Mutex<VecDeque<Reply>>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl ScriptedHub {
    fn with(replies: Vec<Reply>) -> Self {
        Self {
            replies: Mutex::new(VecDeque::from(replies)),
            calls: Arc::default(),
        }
    }
}

impl HubGet for ScriptedHub {
    fn get(
        &self,
        url: &str,
        headers: &Headers,
        query: &[(&str, &str)],
    ) -> Result<HubResponse, TransportError> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(Call {
                url: url.to_owned(),
                headers: headers.clone(),
                query: query
                    .iter()
                    .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                    .collect(),
            });
        let reply = self
            .replies
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front();
        match reply {
            Some(Reply::Response(response)) => Ok(response),
            Some(Reply::EchoModel) => Ok(response(200, &echoed_mapping(url))),
            Some(Reply::Failure) | None => Err(TransportError::Closed),
        }
    }
}

fn echoed_mapping(url: &str) -> String {
    let model = url
        .trim_start_matches("https://huggingface.co/api/models/")
        .to_owned();
    mapping_body("novita", "live", &model)
}

fn response(status_code: u16, body: &str) -> HubResponse {
    HubResponse {
        status_code,
        headers: Headers::from([("x-hub".to_owned(), "1".to_owned())]),
        body: body.as_bytes().to_vec(),
    }
}

fn mapping_body(provider: &str, status: &str, provider_id: &str) -> String {
    format!(
        r#"{{"inferenceProviderMapping":{{"{provider}":{{"status":"{status}","providerId":"{provider_id}"}}}}}}"#
    )
}

fn fetch(hub: ScriptedHub) -> Result<ProviderMapping, HuggingfaceChatError> {
    ProviderMappingCache::new(None, hub).fetch_provider_mapping("org/model")
}

#[test]
fn mapping_fetch_sends_the_documented_get() {
    let hub = ScriptedHub::with(vec![Reply::Response(response(
        200,
        &mapping_body("novita", "live", "org/model"),
    ))]);
    let calls = Arc::clone(&hub.calls);
    let cache = ProviderMappingCache::new(Some(SecretString::from("hub-token")), hub);
    let mapping = cache.fetch_provider_mapping("org/model").unwrap();
    assert_eq!(mapping["novita"].provider_id.as_deref(), Some("org/model"));
    let recorded = calls.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert_eq!(
        recorded[0].url,
        "https://huggingface.co/api/models/org/model"
    );
    assert_eq!(
        recorded[0].query,
        vec![("expand".to_owned(), "inferenceProviderMapping".to_owned())]
    );
    assert_eq!(
        recorded[0].headers.get("Accept"),
        Some(&"application/json".to_owned())
    );
    assert_eq!(
        recorded[0].headers.get("Authorization"),
        Some(&"Bearer hub-token".to_owned())
    );
}

#[test]
fn mapping_fetch_without_a_token_omits_authorization() {
    let hub = ScriptedHub::with(vec![Reply::Response(response(
        200,
        &mapping_body("novita", "live", "org/model"),
    ))]);
    let calls = Arc::clone(&hub.calls);
    ProviderMappingCache::new(None, hub)
        .fetch_provider_mapping("org/model")
        .unwrap();
    assert_eq!(calls.lock().unwrap()[0].headers.get("Authorization"), None);
}

#[test]
fn mapping_fetch_with_an_empty_token_omits_authorization() {
    let hub = ScriptedHub::with(vec![Reply::Response(response(
        200,
        &mapping_body("novita", "live", "org/model"),
    ))]);
    let calls = Arc::clone(&hub.calls);
    ProviderMappingCache::new(Some(SecretString::from("")), hub)
        .fetch_provider_mapping("org/model")
        .unwrap();
    assert_eq!(calls.lock().unwrap()[0].headers.get("Authorization"), None);
}

#[test]
fn mapping_fetch_keeps_the_error_status_and_headers() {
    let hub = ScriptedHub::with(vec![Reply::Response(response(429, "{}"))]);
    let error = fetch(hub).unwrap_err();
    let HuggingfaceChatError::Huggingface(failure) = &error else {
        panic!("unexpected {error:?}");
    };
    assert_eq!(failure.status_code, 429);
    assert_eq!(failure.headers.get("x-hub"), Some(&"1".to_owned()));
    assert!(
        failure
            .message
            .starts_with("Failed to fetch provider mapping: "),
        "{}",
        failure.message
    );
}

#[test]
fn mapping_fetch_redirect_is_a_huggingface_error_too() {
    let hub = ScriptedHub::with(vec![Reply::Response(response(302, ""))]);
    let error = fetch(hub).unwrap_err();
    let HuggingfaceChatError::Huggingface(failure) = &error else {
        panic!("unexpected {error:?}");
    };
    assert_eq!(failure.status_code, 302);
    assert_eq!(failure.headers.get("x-hub"), Some(&"1".to_owned()));
}

#[test]
fn mapping_fetch_transport_failure_is_a_500_with_no_headers() {
    let hub = ScriptedHub::with(vec![Reply::Failure]);
    let error = fetch(hub).unwrap_err();
    let HuggingfaceChatError::Huggingface(failure) = &error else {
        panic!("unexpected {error:?}");
    };
    assert_eq!(failure.status_code, 500);
    assert!(failure.headers.is_empty());
    assert!(
        failure
            .message
            .starts_with("Failed to fetch provider mapping: "),
        "{}",
        failure.message
    );
}

#[test]
fn mapping_fetch_without_the_field_is_not_a_huggingface_error() {
    let hub = ScriptedHub::with(vec![Reply::Response(response(
        200,
        r#"{"id":"org/model"}"#,
    ))]);
    let error = fetch(hub).unwrap_err();
    assert!(
        matches!(error, HuggingfaceChatError::NoProviderMapping { .. }),
        "{error}"
    );
}

#[test]
fn mapping_cache_reuses_a_success() {
    let hub = ScriptedHub::with(vec![Reply::Response(response(
        200,
        &mapping_body("novita", "live", "org/model"),
    ))]);
    let calls = Arc::clone(&hub.calls);
    let cache = ProviderMappingCache::new(None, hub);
    let first = cache.mapping("org/model").unwrap();
    let second = cache.mapping("org/model").unwrap();
    assert_eq!(first, second);
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[test]
fn mapping_cache_keeps_failures_out() {
    let hub = ScriptedHub::with(vec![
        Reply::Response(response(500, "{}")),
        Reply::Response(response(200, &mapping_body("novita", "live", "org/model"))),
    ]);
    let calls = Arc::clone(&hub.calls);
    let cache = ProviderMappingCache::new(None, hub);
    let failed = cache.mapping("org/model");
    let Err(failure) = &failed else {
        panic!("the first fetch should fail");
    };
    assert!(matches!(failure, HuggingfaceChatError::Huggingface(_)));
    let Ok(mapping) = cache.mapping("org/model") else {
        panic!("the second fetch should succeed");
    };
    assert_eq!(mapping["novita"].provider_id.as_deref(), Some("org/model"));
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[test]
fn mapping_cache_evicts_the_least_recently_used_entry() {
    let hub = ScriptedHub::with(repeat_n(Reply::EchoModel, 200_usize).collect());
    let calls = Arc::clone(&hub.calls);
    let cache = ProviderMappingCache::new(None, hub);
    (0_u32..128_u32).for_each(|index| drop(cache.mapping(&format!("org/m{index}")).unwrap()));
    drop(cache.mapping("org/m0").unwrap());
    drop(cache.mapping("org/m128").unwrap());
    assert_eq!(calls.lock().unwrap().len(), 129);
    drop(cache.mapping("org/m0").unwrap());
    assert_eq!(calls.lock().unwrap().len(), 129);
    drop(cache.mapping("org/m1").unwrap());
    assert_eq!(calls.lock().unwrap().len(), 130);
}

#[test]
fn mapping_decode_reads_entries() {
    let mapping =
        decode_provider_mapping(mapping_body("novita", "live", "org/model").as_bytes()).unwrap();
    assert_eq!(mapping["novita"].status.as_deref(), Some("live"));
    assert_eq!(mapping["novita"].provider_id.as_deref(), Some("org/model"));
}

#[test]
fn mapping_decode_missing_field_is_typed() {
    let error = decode_provider_mapping(br#"{"id":"org/model"}"#).unwrap_err();
    assert!(
        matches!(error, HuggingfaceChatError::MappingMissing),
        "{error}"
    );
}

#[test]
fn mapping_decode_null_field_is_typed() {
    let error = decode_provider_mapping(br#"{"inferenceProviderMapping":null}"#).unwrap_err();
    assert!(
        matches!(error, HuggingfaceChatError::MappingMissing),
        "{error}"
    );
}

#[test]
fn mapping_decode_rejects_malformed_json() {
    let error = decode_provider_mapping(b"{").unwrap_err();
    assert!(
        matches!(error, HuggingfaceChatError::MappingDecode { .. }),
        "{error}"
    );
}

#[test]
fn mapping_decode_rejects_over_one_mebibyte() {
    let bytes = vec![b' '; 1024 * 1024 + 1];
    let error = decode_provider_mapping(&bytes).unwrap_err();
    assert!(
        matches!(error, HuggingfaceChatError::MappingTooLarge { .. }),
        "{error}"
    );
}

#[test]
fn mapping_decode_accepts_a_body_of_exactly_one_mebibyte() {
    let mut bytes = mapping_body("novita", "live", "org/model").into_bytes();
    bytes.resize(1024 * 1024, b' ');
    let mapping = decode_provider_mapping(&bytes).unwrap();
    assert_eq!(mapping["novita"].provider_id.as_deref(), Some("org/model"));
}

#[test]
fn mapping_decode_keeps_unknown_entry_fields_out_of_the_shape() {
    let body = r#"{"inferenceProviderMapping":{"novita":{"status":"live","providerId":"org/model","task":"text-generation"}}}"#;
    let mapping = decode_provider_mapping(body.as_bytes()).unwrap();
    assert_eq!(mapping["novita"].provider_id.as_deref(), Some("org/model"));
}

#[test]
fn replays_the_provider_mapping_corpus() {
    let corpus =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fuzz/corpus/huggingface_provider_mapping");
    let mut paths = read_dir(corpus)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "no provider mapping corpus seeds");
    while let Some(path) = paths.pop() {
        let bytes = read(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let decoded = decode_provider_mapping(&bytes);
        if name.starts_with("valid-") {
            let Ok(_mapping) = &decoded else {
                panic!("{name}: {decoded:?}");
            };
        } else {
            let Err(_error) = &decoded else {
                panic!("{name}: {decoded:?}");
            };
        }
    }
}
