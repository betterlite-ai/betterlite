#![cfg(test)]

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use betterlite_openai::chat::response::ChatResponse;
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::SecretString;

use super::environment::SapChatConfig;
use super::error::{CREDENTIALS_STATUS, SapChatError};
use super::params::{SUPPORTED_OPENAI_PARAMS, get_supported_openai_params};
use super::prompt_module::build_prompt_module;
use super::request::transform_request;
use super::response::{
    PostCall, PostCallLogger, ResponseInput, get_model_response_iterator, transform_response,
};
use crate::sap_auth::environment::SapEnvironment;
use crate::sap_stream::Mode;

fn json(text: &str) -> Json {
    serde_json::from_str(text).unwrap()
}

fn object(text: &str) -> Object {
    serde_json::from_str(text).unwrap()
}

fn list(text: &str) -> Vec<Json> {
    serde_json::from_str(text).unwrap()
}

fn at<'tree>(value: &'tree Json, path: &[&str]) -> &'tree Json {
    let mut current = value;
    for name in path {
        current = match current {
            Json::Object(fields) => fields.get(*name).unwrap(),
            Json::List(items) => &items[name.parse::<usize>().unwrap()],
            Json::Null | Json::Bool(_) | Json::Number(_) | Json::Text(_) => {
                panic!("{name} is not under an object or list")
            }
        };
    }
    current
}

#[derive(Default)]
struct FakeEnvironment {
    vars: BTreeMap<String, String>,
    files: BTreeMap<PathBuf, String>,
    home: PathBuf,
}

impl FakeEnvironment {
    fn with_var(mut self, name: &str, value: &str) -> Self {
        self.vars.insert(name.to_owned(), value.to_owned());
        self
    }

    fn home(mut self, home: &str) -> Self {
        self.home = PathBuf::from(home);
        self
    }
}

impl SapEnvironment for FakeEnvironment {
    fn var(&self, name: &str) -> Option<String> {
        self.vars.get(name).cloned()
    }

    fn file_exists(&self, path: &Path) -> bool {
        self.files.contains_key(path)
    }

    fn read_file(&self, path: &Path) -> Result<String, io::Error> {
        self.files.get(path).map_or_else(
            || Err(io::Error::from(io::ErrorKind::NotFound)),
            |contents| Ok(contents.clone()),
        )
    }

    fn home_dir(&self) -> PathBuf {
        self.home.clone()
    }
}

fn config() -> SapChatConfig {
    SapChatConfig {
        token: SecretString::from("Bearer case-token"),
        resource_group: String::from("rg1"),
        deployment_url: String::from("https://dep.example.com/v2/"),
    }
}

const SERVICE_KEY: &str = r#"{"clientid":"id","clientsecret":"secret","url":"https://auth.example.com/oauth/token","serviceurls":{"AI_API_URL":"https://api.example.com/v2"},"resource_group":"key-rg"}"#;

#[test]
fn supported_params_list_the_full_set_in_order() {
    assert_eq!(
        get_supported_openai_params("anthropic--claude-3.5-sonnet"),
        SUPPORTED_OPENAI_PARAMS
    );
}

#[rstest]
#[case::amazon("amazon.titan-text-lite-v1", &["response_format", "tool_choice"])]
#[case::cohere("cohere.command-r-plus", &["response_format"])]
#[case::alephalpha("alephalpha-luminous-base", &["response_format"])]
#[case::gpt_4_exact("gpt-4", &["response_format"])]
#[case::gemini("gemini-1.5-pro", &["tool_choice"])]
fn supported_params_drop_the_prefixes_that_do_not_support_them(
    #[case] model: &str,
    #[case] removed: &[&str],
) {
    let params = get_supported_openai_params(model);
    for name in removed {
        assert!(!params.contains(name), "{model} must not list {name}");
    }
    assert_eq!(
        params.len(),
        SUPPORTED_OPENAI_PARAMS.len().saturating_sub(removed.len())
    );
}

#[rstest]
#[case::gpt_4o_keeps_it("gpt-4o")]
#[case::upper_case_gpt_keeps_it("GPT-4")]
#[case::sap_prefix_keeps_it("sap/gpt-4")]
fn supported_params_keep_response_format_for_names_outside_the_list(#[case] model: &str) {
    let params = get_supported_openai_params(model);
    assert!(params.contains(&"response_format"));
    assert!(params.contains(&"tool_choice"));
}

#[test]
fn request_builds_the_orchestration_body() {
    let messages = list(
        r#"[
            {"role":"system","content":"be nice"},
            {"role":"user","content":"hi"},
            {"role":"assistant","content":"hello"},
            {"role":"tool","tool_call_id":"t1","content":"result"},
            {"role":"developer","content":"dev"}
        ]"#,
    );
    let params = object(
        r#"{
            "deployment_url":"https://ignored.example.com",
            "stream":true,
            "tool_choice":"auto",
            "temperature":0.5,
            "max_tokens":12
        }"#,
    );
    let body = transform_request("anthropic--claude-3.5-sonnet", &messages, &params).unwrap();
    assert_eq!(
        body,
        json(
            r#"{"config":{"modules":{"prompt_templating":{
                "prompt":{"template":[
                    {"role":"system","content":"be nice"},
                    {"role":"user","content":"hi"},
                    {"role":"assistant","content":"hello"},
                    {"role":"tool","tool_call_id":"t1","content":"result"},
                    {"role":"developer","content":"dev"}
                ]},
                "model":{
                    "name":"anthropic--claude-3.5-sonnet",
                    "version":"latest",
                    "params":{"temperature":0.5,"max_tokens":12}
                }
            }}}}"#
        )
    );
}

#[test]
fn request_keeps_placeholder_values_only_when_they_are_not_null() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let with_values = transform_request(
        "m",
        &messages,
        &object(r#"{"placeholder_values":{"q":"v"}}"#),
    )
    .unwrap();
    assert_eq!(
        at(&with_values, &["placeholder_values"]),
        &json(r#"{"q":"v"}"#)
    );
    let with_empty =
        transform_request("m", &messages, &object(r#"{"placeholder_values":{}}"#)).unwrap();
    assert_eq!(at(&with_empty, &["placeholder_values"]), &json("{}"));
    let without =
        transform_request("m", &messages, &object(r#"{"placeholder_values":null}"#)).unwrap();
    assert!(matches!(&without, Json::Object(fields) if !fields.contains_key("placeholder_values")));
}

#[test]
fn stream_options_keep_only_chunk_size_and_delimiters() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "m",
        &messages,
        &object(r#"{"stream_options":{"chunk_size":10,"delimiters":["\n"],"include_usage":true}}"#),
    )
    .unwrap();
    assert_eq!(
        at(&body, &["config", "stream"]),
        &json(r#"{"chunk_size":10,"delimiters":["\n"]}"#)
    );
}

#[rstest]
#[case::absent("{}")]
#[case::empty(r#"{"stream_options":{}}"#)]
#[case::unknown_keys_only(r#"{"stream_options":{"include_usage":true}}"#)]
#[case::null(r#"{"stream_options":null}"#)]
fn stream_is_added_only_when_a_kept_key_is_present(#[case] params: &str) {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request("m", &messages, &object(params)).unwrap();
    assert!(
        matches!(&body, Json::Object(fields) if matches!(fields.get("config"), Some(Json::Object(config)) if !config.contains_key("stream"))),
        "no config.stream for {params}"
    );
}

#[test]
fn stream_options_keep_an_explicit_null_chunk_size() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(
        "m",
        &messages,
        &object(r#"{"stream_options":{"chunk_size":null}}"#),
    )
    .unwrap();
    assert_eq!(
        at(&body, &["config", "stream"]),
        &json(r#"{"chunk_size":null}"#)
    );
}

#[test]
fn tools_and_response_format_land_in_the_prompt() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(
        r#"{
            "tools":[{"type":"function","function":{"name":"f","parameters":{"properties":{"a":{"type":"string"}}}}}],
            "response_format":{"type":"json_object"}
        }"#,
    );
    let body = transform_request("m", &messages, &params).unwrap();
    let prompt = at(&body, &["config", "modules", "prompt_templating", "prompt"]);
    assert_eq!(
        at(prompt, &["response_format"]),
        &json(r#"{"type":"json_object"}"#)
    );
    assert_eq!(
        at(prompt, &["tools", "0", "function", "name"]),
        &json(r#""f""#)
    );
    assert_eq!(
        at(prompt, &["tools", "0", "function", "strict"]),
        &json("false")
    );
    assert_eq!(
        at(prompt, &["tools", "0", "function", "parameters"]),
        &json(r#"{"type":"object","properties":{"a":{"type":"string"}}}"#)
    );
}

#[test]
fn a_json_schema_response_format_goes_through_the_schema_model() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(
        r#"{"response_format":{"type":"json_schema","json_schema":{"name":"s","schema":{"type":"object"}}}}"#,
    );
    let body = transform_request("m", &messages, &params).unwrap();
    // LiteLLM dumps ResponseFormatJSONSchema with exclude_unset, so an unset strict stays absent
    assert_eq!(
        at(
            &body,
            &[
                "config",
                "modules",
                "prompt_templating",
                "prompt",
                "response_format",
            ],
        ),
        &json(r#"{"type":"json_schema","json_schema":{"name":"s","schema":{"type":"object"}}}"#)
    );
}

#[rstest]
#[case::absent("{}")]
#[case::no_type(r#"{"response_format":{}}"#)]
#[case::empty_type(r#"{"response_format":{"type":""}}"#)]
#[case::null(r#"{"response_format":null}"#)]
fn a_response_format_without_a_type_is_dropped(#[case] params: &str) {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request("m", &messages, &object(params)).unwrap();
    let prompt = at(&body, &["config", "modules", "prompt_templating", "prompt"]);
    assert!(matches!(prompt, Json::Object(fields) if !fields.contains_key("response_format")));
}

#[rstest]
#[case::gpt("gpt-4o", false)]
#[case::gpt_41("gpt-4.1", false)]
#[case::anthropic("anthropic--claude-3.5-sonnet", true)]
fn strict_is_dropped_for_gpt_models_only(#[case] model: &str, #[case] kept: bool) {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request(model, &messages, &object(r#"{"strict":true}"#)).unwrap();
    let params = at(
        &body,
        &["config", "modules", "prompt_templating", "model", "params"],
    );
    assert_eq!(
        matches!(params, Json::Object(fields) if fields.contains_key("strict")),
        kept
    );
}

#[test]
fn placeholder_defaults_are_written_as_defaults_only_when_non_empty() {
    let module =
        build_prompt_module("m", &[], object(r#"{"placeholder_defaults":{"q":"d"}}"#)).unwrap();
    assert_eq!(
        at(&module, &["prompt_templating", "prompt", "defaults"]),
        &json(r#"{"q":"d"}"#)
    );
    let empty = build_prompt_module("m", &[], object(r#"{"placeholder_defaults":{}}"#)).unwrap();
    assert!(
        matches!(at(&empty, &["prompt_templating", "prompt"]), Json::Object(fields) if !fields.contains_key("defaults"))
    );
    let absent = build_prompt_module("m", &[], object(r#"{"placeholder_defaults":null}"#)).unwrap();
    assert!(
        matches!(at(&absent, &["prompt_templating", "prompt"]), Json::Object(fields) if !fields.contains_key("defaults"))
    );
}

#[test]
fn optional_modules_move_to_the_module_level() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(
        r#"{
            "translation":{"input":{"type":"sap_document_translation","config":{"source_language":"en","target_language":"de"}}},
            "grounding":{"type":"document_grounding_service","config":{"placeholders":{"input":["q"],"output":"o"}}},
            "masking":{"providers":[{"type":"sap_data_privacy_integration","method":"anonymization","entities":[{"type":"profile-email"}]}]},
            "filtering":{"input":{"filters":[{"type":"azure_content_safety","config":{"hate":2}}]}}
        }"#,
    );
    let body = transform_request("m", &messages, &params).unwrap();
    let module = at(&body, &["config", "modules"]);
    assert_eq!(
        at(module, &["grounding", "type"]),
        &json(r#""document_grounding_service""#)
    );
    assert_eq!(
        at(
            module,
            &["translation", "input", "config", "target_language"]
        ),
        &json(r#""de""#)
    );
    assert_eq!(
        at(module, &["masking", "providers", "0", "type"]),
        &json(r#""sap_data_privacy_integration""#)
    );
    assert_eq!(
        at(
            module,
            &["filtering", "input", "filters", "0", "config", "hate"]
        ),
        &json("2")
    );
}

#[test]
fn a_null_optional_module_stays_in_the_model_params() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let body = transform_request("m", &messages, &object(r#"{"grounding":null}"#)).unwrap();
    let module = at(&body, &["config", "modules"]);
    assert!(matches!(module, Json::Object(fields) if !fields.contains_key("grounding")));
    assert_eq!(
        at(
            module,
            &["prompt_templating", "model", "params", "grounding"]
        ),
        &Json::Null
    );
}

#[test]
fn fallback_modules_are_built_from_their_own_params() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(
        r#"{"fallback_sap_modules":[
            {"model":"sap/gpt-4o","messages":[{"role":"user","content":"fb"}],"temperature":0.1,"strict":true}
        ]}"#,
    );
    let body = transform_request("anthropic--claude-3.5-sonnet", &messages, &params).unwrap();
    let modules = at(&body, &["config", "modules"]);
    assert_eq!(
        at(modules, &["0", "prompt_templating", "model", "name"]),
        &json(r#""anthropic--claude-3.5-sonnet""#)
    );
    assert_eq!(
        at(modules, &["1", "prompt_templating", "model", "name"]),
        &json(r#""gpt-4o""#)
    );
    assert_eq!(
        at(modules, &["1", "prompt_templating", "model", "params"]),
        &json(r#"{"temperature":0.1}"#)
    );
    assert_eq!(
        at(modules, &["1", "prompt_templating", "prompt", "template"]),
        &json(r#"[{"role":"user","content":"fb"}]"#)
    );
}

#[test]
fn model_version_moves_to_the_module_version_field() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(
        r#"{
            "model_version":"2024-05-13",
            "temperature":0.1,
            "fallback_sap_modules":[
                {"model":"sap/other","model_version":"2024-06-14","temperature":0.2}
            ]
        }"#,
    );
    let body = transform_request("m", &messages, &params).unwrap();
    let modules = at(&body, &["config", "modules"]);
    assert_eq!(
        at(modules, &["0", "prompt_templating", "model", "version"]),
        &json(r#""2024-05-13""#)
    );
    assert_eq!(
        at(modules, &["0", "prompt_templating", "model", "params"]),
        &json(r#"{"temperature":0.1}"#)
    );
    assert_eq!(
        at(modules, &["1", "prompt_templating", "model", "version"]),
        &json(r#""2024-06-14""#)
    );
    assert_eq!(
        at(modules, &["1", "prompt_templating", "model", "params"]),
        &json(r#"{"temperature":0.2}"#)
    );
}

#[test]
fn a_fallback_module_without_messages_gets_an_empty_template() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let params = object(r#"{"fallback_sap_modules":[{"model":"other"}]}"#);
    let body = transform_request("m", &messages, &params).unwrap();
    assert_eq!(
        at(
            &body,
            &[
                "config",
                "modules",
                "1",
                "prompt_templating",
                "prompt",
                "template"
            ]
        ),
        &json("[]")
    );
}

#[rstest]
#[case::missing(r#"{"fallback_sap_modules":[{"messages":[]}]}"#)]
#[case::null(r#"{"fallback_sap_modules":[{"model":null}]}"#)]
fn a_fallback_module_without_a_model_is_rejected(#[case] params: &str) {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let error = transform_request("m", &messages, &object(params)).unwrap_err();
    assert!(matches!(error, SapChatError::Fallback(_)));
    assert_eq!(error.status(), None);
}

#[test]
fn a_fallback_module_that_is_not_an_object_is_rejected() {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let error = transform_request("m", &messages, &object(r#"{"fallback_sap_modules":["x"]}"#))
        .unwrap_err();
    assert!(matches!(error, SapChatError::Fallback(_)));
}

#[test]
fn environment_returns_only_the_four_sap_headers() {
    let call = config().prepare().unwrap();
    let headers = call.headers;
    assert_eq!(headers.len(), 4);
    assert_eq!(headers.get("authorization").unwrap(), "Bearer case-token");
    assert_eq!(headers.get("ai-resource-group").unwrap(), "rg1");
    assert_eq!(headers.get("content-type").unwrap(), "application/json");
    assert_eq!(headers.get("ai-client-type").unwrap(), "LiteLLM");
    assert_eq!(call.url, "https://dep.example.com/v2//v2/completion");
}

#[test]
fn setup_returns_none_when_no_service_key_is_given() {
    let environment = FakeEnvironment::default().home("/nowhere");
    let absent = SapChatConfig::setup(None, &environment).unwrap();
    assert!(absent.is_none());
    let empty = Json::Text(String::new());
    let empty_key = SapChatConfig::setup(Some(&empty), &environment).unwrap();
    assert!(
        empty_key.is_none(),
        "an empty service key does not run the setup"
    );
}

#[test]
fn a_service_key_supplies_the_resource_group_for_the_headers() {
    let environment = FakeEnvironment::default().home("/nowhere");
    let service_key = Json::Text(SERVICE_KEY.to_owned());
    let credentials = SapChatConfig::setup(Some(&service_key), &environment)
        .unwrap()
        .unwrap();
    assert_eq!(credentials.resource_group, "key-rg");
    let call = SapChatConfig {
        token: SecretString::from("Bearer case-token"),
        resource_group: credentials.resource_group,
        deployment_url: String::from("https://dep.example.com/v2"),
    }
    .prepare()
    .unwrap();
    assert_eq!(call.headers.get("ai-resource-group").unwrap(), "key-rg");
    assert_ne!(call.headers.get("ai-resource-group").unwrap(), "rg1");
}

#[test]
fn environment_turns_missing_credentials_into_a_400() {
    let environment = FakeEnvironment::default().home("/nowhere");
    let empty_key = Json::Text(String::from("{}"));
    let error = SapChatConfig::setup(Some(&empty_key), &environment).unwrap_err();
    assert!(matches!(error, SapChatError::Orchestration(_)));
    assert_eq!(error.status(), Some(CREDENTIALS_STATUS));
}

#[test]
fn environment_does_not_wrap_other_setup_failures() {
    let environment = FakeEnvironment::default()
        .home("/nowhere")
        .with_var("AICORE_CONFIG", "/missing/config.json");
    let service_key = Json::Text(String::from("x"));
    let error = SapChatConfig::setup(Some(&service_key), &environment).unwrap_err();
    assert!(matches!(error, SapChatError::Auth(_)));
    assert_eq!(error.status(), None);
}

#[test]
fn environment_turns_incomplete_credentials_into_a_400() {
    let environment = FakeEnvironment::default()
        .home("/nowhere")
        .with_var("AICORE_CLIENT_ID", "id")
        .with_var("AICORE_AUTH_URL", "https://auth.example.com/oauth/token")
        .with_var("AICORE_BASE_URL", "https://api.example.com/v2");
    let with_key =
        SapChatConfig::setup(Some(&json(r#"{"client_id":"id"}"#)), &environment).unwrap_err();
    assert_eq!(with_key.status(), Some(CREDENTIALS_STATUS));
}

type RecordedCalls = Vec<(Vec<Json>, Option<String>, String, Json)>;

struct Recorder {
    calls: Mutex<RecordedCalls>,
}

impl Recorder {
    fn new() -> Self {
        Self {
            calls: Mutex::new(Vec::new()),
        }
    }
}

impl PostCallLogger for Recorder {
    fn post_call(&self, call: PostCall<'_>) {
        self.calls.lock().unwrap().push((
            call.input.to_vec(),
            call.api_key.map(str::to_owned),
            call.original_response.to_owned(),
            call.request_data.clone(),
        ));
    }
}

const RAW_TEMPLATE: &str = r#"{"final_result":{"id":"chatcmpl-1","object":"chat.completion","created":1700000000,"model":"anthropic--claude-3.5-sonnet","choices":[{"index":0,"message":{"role":"assistant","content":CONTENT},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":4,"total_tokens":7}}}"#;

fn raw_response(content: &str) -> String {
    RAW_TEMPLATE.replace("CONTENT", &serde_json::to_string(content).unwrap())
}

fn response(model: &str, format: &str, content: &str, logger: &Recorder) -> ChatResponse {
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let request_data = json(r#"{"config":{"modules":{}}}"#);
    let response_format = json(format);
    transform_response(
        &ResponseInput {
            model,
            messages: &messages,
            api_key: Some("sk-test"),
            raw_response: &raw_response(content),
            request_data: &request_data,
            response_format: Some(&response_format),
        },
        logger,
    )
    .unwrap()
}

#[test]
fn response_parses_final_result_and_logs_the_post_call() {
    let logger = Recorder::new();
    let parsed = response(
        "anthropic--claude-3.5-sonnet",
        r#"{"type":"text"}"#,
        "hello",
        &logger,
    );
    assert_eq!(parsed.id, "chatcmpl-1");
    assert_eq!(parsed.choices[0].message.content.as_deref(), Some("hello"));
    let calls = logger.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, list(r#"[{"role":"user","content":"hi"}]"#));
    assert_eq!(calls[0].1.as_deref(), Some("sk-test"));
    assert_eq!(calls[0].2, raw_response("hello"));
    assert_eq!(calls[0].3, json(r#"{"config":{"modules":{}}}"#));
}

#[rstest]
#[case::json_tag("```json\n{\"a\": 1}\n```", "{\"a\": 1}")]
#[case::plain_fence("```\n{\"a\": 1}\n```", "{\"a\": 1}")]
#[case::spaces("  ```json\n{\"a\": 1}\n```  ", "{\"a\": 1}")]
#[case::no_whitespace("```json{\"a\": 1}```", "{\"a\": 1}")]
#[case::no_fence("{\"a\": 1}", "{\"a\": 1}")]
#[case::trailing_text("```json\n{\"a\": 1}\n```tail", "```json\n{\"a\": 1}\n```tail")]
fn response_strips_a_markdown_fence_for_anthropic_json_replies(
    #[case] content: &str,
    #[case] expected: &str,
) {
    let logger = Recorder::new();
    let parsed = response(
        "anthropic--claude-3.5-sonnet",
        r#"{"type":"json_object"}"#,
        content,
        &logger,
    );
    assert_eq!(parsed.choices[0].message.content.as_deref(), Some(expected));
}

#[test]
fn response_strips_a_fence_for_a_json_schema_reply() {
    let logger = Recorder::new();
    let parsed = response(
        "anthropic--claude-3.5-sonnet",
        r#"{"type":"json_schema"}"#,
        "```json\n{}\n```",
        &logger,
    );
    assert_eq!(parsed.choices[0].message.content.as_deref(), Some("{}"));
}

#[rstest]
#[case::gpt_model("gpt-4o", r#"{"type":"json_object"}"#)]
#[case::text_format("anthropic--claude-3.5-sonnet", r#"{"type":"text"}"#)]
#[case::missing_format("anthropic--claude-3.5-sonnet", "{}")]
#[case::null_format("anthropic--claude-3.5-sonnet", "null")]
fn response_keeps_the_fence_when_the_gate_does_not_hold(#[case] model: &str, #[case] format: &str) {
    let logger = Recorder::new();
    let parsed = response(model, format, "```json\n{}\n```", &logger);
    assert_eq!(
        parsed.choices[0].message.content.as_deref(),
        Some("```json\n{}\n```")
    );
}

#[test]
fn response_leaves_an_empty_content_alone() {
    let logger = Recorder::new();
    let parsed = response(
        "anthropic--claude-3.5-sonnet",
        r#"{"type":"json_object"}"#,
        "",
        &logger,
    );
    assert_eq!(parsed.choices[0].message.content.as_deref(), Some(""));
}

#[test]
fn response_without_a_final_result_fails() {
    let logger = Recorder::new();
    let messages = list(r#"[{"role":"user","content":"hi"}]"#);
    let request_data = json("{}");
    let error = transform_response(
        &ResponseInput {
            model: "m",
            messages: &messages,
            api_key: None,
            raw_response: r#"{"other": true}"#,
            request_data: &request_data,
            response_format: None,
        },
        &logger,
    )
    .unwrap_err();
    assert!(matches!(error, SapChatError::Response(_)));
    assert_eq!(
        logger.calls.lock().unwrap().len(),
        1,
        "the post call comes first"
    );
}

#[rstest]
#[case::sync(true, Mode::Sync)]
#[case::asynchronous(false, Mode::Async)]
fn response_iterator_mode_follows_the_sync_stream_flag(
    #[case] sync_stream: bool,
    #[case] expected: Mode,
) {
    assert_eq!(get_model_response_iterator(sync_stream), expected);
}
