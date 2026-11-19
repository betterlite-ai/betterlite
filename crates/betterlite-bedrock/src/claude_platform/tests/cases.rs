#![cfg_attr(
    test,
    expect(
        clippy::too_many_lines,
        reason = "each named case states one LiteLLM behavior as a single scenario"
    )
)]

use std::collections::BTreeSet;

use betterlite_anthropic::headers::AnthropicHeaders;
use betterlite_anthropic::stream_decode::TransportChunk;
use betterlite_anthropic::stream_decode::error::ReceiveError;
use betterlite_config::sections::claude_platform_inputs::{PlatformEnvironment, PlatformInputs};
use betterlite_models::decisions::Json;
use betterlite_prompts::arguments::json::Json as PromptJson;
use indexmap::IndexMap;
use secrecy::SecretString;
use serde_json::json;

use super::support::{
    CounterError, Counting, FixedIds, MAX_EVENT_BYTES, Records, defaults, stream_chunks,
};
use crate::claude_platform::chat::{ChatHeadersRequest, ChatRequest};
use crate::claude_platform::convert;
use crate::claude_platform::error::{
    ChatValidationError, NativeHeaderError, NativeTransformError, WorkspaceError,
};
use crate::claude_platform::native::{
    NativeHeaders, NativeHeadersRequest, NativeRequest, validate_environment as validate_native,
};
use crate::claude_platform::stream::{StreamFlags, get_model_response_iterator};
use crate::claude_platform::{FILTERS_ANTHROPIC_BETAS, WORKSPACE_HEADER, WorkspaceSources};
use crate::claude_platform_inputs::body::PlatformWarning;

const MISSING_WORKSPACE: &str = "Missing workspace ID for Claude Platform on AWS. Pass `workspace_id` or configure the provider workspace setting.";
const STREAM_ID: &str = "00000000-0000-4000-8000-000000000001";

fn json_map(value: serde_json::Value) -> IndexMap<String, Json> {
    serde_json::from_value(value).expect("the case params decode")
}

fn json_list(value: serde_json::Value) -> Vec<Json> {
    serde_json::from_value(value).expect("the case messages decode")
}

fn headers(value: serde_json::Value) -> IndexMap<String, String> {
    serde_json::from_value(value).expect("the case headers decode")
}

fn header<'headers>(
    values: &'headers IndexMap<String, String>,
    name: &str,
) -> Option<&'headers str> {
    values.get(name).map(String::as_str)
}

fn beta_set(values: &IndexMap<String, String>) -> BTreeSet<String> {
    let Some(value) = values.get("anthropic-beta") else {
        return BTreeSet::new();
    };
    value
        .split(',')
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
        .collect()
}

fn expected_betas(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

// One request the cases vary; every field a validation or transform reads lives here.
#[derive(Default)]
struct Case {
    model: &'static str,
    messages: Vec<Json>,
    optional_params: IndexMap<String, Json>,
    litellm_params: IndexMap<String, Json>,
    caller_headers: IndexMap<String, String>,
    api_key: Option<SecretString>,
    api_base: Option<String>,
    deployment: PlatformInputs,
    global: PlatformInputs,
    environment: PlatformEnvironment,
}

impl Case {
    fn chat_headers(&self, records: &Records) -> Result<AnthropicHeaders, ChatValidationError> {
        records.chat_headers(&ChatHeadersRequest {
            model: self.model,
            messages: &self.messages,
            optional_params: &self.optional_params,
            caller_headers: &self.caller_headers,
            api_key: self.api_key.as_ref(),
            workspace: WorkspaceSources {
                deployment: &self.deployment,
                global: &self.global,
                environment: &self.environment,
            },
        })
    }

    fn native_headers(&self) -> Result<NativeHeaders, NativeHeaderError> {
        let messages =
            convert::prompt_messages(&self.messages).expect("the case messages round trip");
        let optional_params: IndexMap<String, PromptJson> =
            convert::mapped(&self.optional_params).expect("the case params round trip");
        validate_native(&NativeHeadersRequest {
            model: self.model,
            caller_headers: &self.caller_headers,
            messages: &messages,
            optional_params: &optional_params,
            litellm_params: &self.litellm_params,
            api_key: self.api_key.as_ref(),
            api_base: self.api_base.as_deref(),
            workspace: WorkspaceSources {
                deployment: &self.deployment,
                global: &self.global,
                environment: &self.environment,
            },
        })
    }
}

fn request_case() -> Case {
    Case {
        model: "claude-sonnet-4-5",
        messages: json_list(json!([{"role": "user", "content": "hi"}])),
        optional_params: IndexMap::new(),
        litellm_params: IndexMap::new(),
        caller_headers: IndexMap::new(),
        api_key: None,
        api_base: None,
        deployment: PlatformInputs::default(),
        global: PlatformInputs::default(),
        environment: PlatformEnvironment::default(),
    }
}

// Both bodies carry the token budget and the filtered keys, so one helper reads the shared assertions.
fn inspect_body<Body>(body: &Body, model: &str, max_tokens: &str) -> serde_json::Value
where
    Body: serde::Serialize,
{
    let value = serde_json::to_value(body).expect("the body encodes");
    let encoded = value.to_string();
    assert_eq!(
        value.get("model").and_then(serde_json::Value::as_str),
        Some(model),
        "{encoded}"
    );
    assert!(encoded.contains(max_tokens), "{encoded}");
    assert!(value.get("context_management").is_none(), "{encoded}");
    value
}

// B2211's alias chain, read through both validations, and the one 401 it raises.
pub(crate) fn workspace_id_required() {
    let records = Records::empty();
    let mut case = request_case();
    case.optional_params = json_map(json!({"workspace_id": "request-workspace"}));
    case.caller_headers = headers(json!({"anthropic-workspace-id": "caller-workspace"}));
    case.deployment.workspace_id = Some("deployment-workspace".to_owned());
    case.global.workspace_id = Some("global-workspace".to_owned());
    case.environment.anthropic_aws_workspace_id = Some(SecretString::from("environment-workspace"));
    let chat = case
        .chat_headers(&records)
        .expect("the request workspace resolves");
    assert_eq!(
        header(&chat.values, WORKSPACE_HEADER),
        Some("request-workspace")
    );
    let native = case
        .native_headers()
        .expect("the native validation reads the same workspace");
    assert_eq!(
        header(&native.headers, WORKSPACE_HEADER),
        Some("request-workspace")
    );

    // Without a request alias the deployment value answers, and the module value is its fallback.
    case.optional_params = IndexMap::new();
    let chat = case.chat_headers(&records).expect("the deployment value");
    assert_eq!(
        header(&chat.values, WORKSPACE_HEADER),
        Some("deployment-workspace")
    );
    case.deployment = PlatformInputs::default();
    let chat = case.chat_headers(&records).expect("the module value");
    assert_eq!(
        header(&chat.values, WORKSPACE_HEADER),
        Some("global-workspace")
    );
    case.global = PlatformInputs::default();

    // The environment chain reads the AWS alias first and skips an empty one.
    case.environment.anthropic_aws_workspace_id = Some(SecretString::from(""));
    case.environment.anthropic_workspace_id = Some(SecretString::from("second-environment"));
    let chat = case
        .chat_headers(&records)
        .expect("the second environment value");
    assert_eq!(
        header(&chat.values, WORKSPACE_HEADER),
        Some("second-environment")
    );
    case.environment.anthropic_workspace_id = None;

    // An empty request selector is falsy, so the environment answers instead.
    case.optional_params = json_map(json!({"workspace_id": ""}));
    let chat = case.chat_headers(&records).expect("the environment value");
    assert_eq!(
        header(&chat.values, WORKSPACE_HEADER),
        Some("environment-workspace")
    );

    // Python's `or` chain keeps its last operand, so a falsy final alias is sent as an empty header.
    case.optional_params = IndexMap::new();
    case.global.anthropic_hyphen_workspace_id = Some(String::new());
    let chat = case
        .chat_headers(&records)
        .expect("the empty selector renders");
    assert_eq!(header(&chat.values, WORKSPACE_HEADER), Some(""));
    let native = case
        .native_headers()
        .expect("the empty selector renders for native too");
    assert_eq!(header(&native.headers, WORKSPACE_HEADER), Some(""));

    // Nothing configured anywhere is the only failure.
    let case = request_case();
    let failure = case
        .chat_headers(&records)
        .expect_err("a missing workspace fails");
    assert_eq!(failure.status_code(), Some(401));
    assert_eq!(failure.to_string(), MISSING_WORKSPACE);
    let ChatValidationError::Workspace(workspace @ WorkspaceError::Missing { model }) = &failure
    else {
        panic!("the chat validation reports the missing workspace: {failure:?}");
    };
    assert_eq!(model, "claude-sonnet-4-5");
    assert_eq!(workspace.model(), Some("claude-sonnet-4-5"));
    let failure = case
        .native_headers()
        .expect_err("a missing workspace fails for native too");
    assert_eq!(failure.status_code(), Some(401));
    assert_eq!(failure.to_string(), MISSING_WORKSPACE);
}

// B570 over the caller's headers, with the feature predicates and the caller's beta list.
pub(crate) fn chat_platform_headers() {
    let records = Records::empty();
    let mut case = request_case();
    case.optional_params = json_map(json!({
        "output_config": {"effort": "high"},
        "mcp_servers": [{"type": "url", "url": "https://example.test/mcp"}],
        "container": {"skills": [{"type": "anthropic", "skill_id": "pdf"}]},
        "tools": [{"type": "code_execution_20250825", "name": "code_execution"}],
        "workspace_id": "request-workspace",
    }));
    case.caller_headers = headers(json!({
        "anthropic-beta": "alpha,beta",
        "x-api-key": "caller-key",
        "x-custom": "kept",
    }));
    case.api_key = Some(SecretString::from("request-key"));
    let chat = case.chat_headers(&records).expect("the chat headers build");
    assert_eq!(header(&chat.values, "x-api-key"), Some("request-key"));
    assert_eq!(header(&chat.values, "x-custom"), Some("kept"));
    assert_eq!(
        header(&chat.values, WORKSPACE_HEADER),
        Some("request-workspace")
    );
    assert_eq!(
        header(&chat.values, "anthropic-version"),
        Some("2023-06-01")
    );
    assert_eq!(
        header(&chat.values, "content-type"),
        Some("application/json")
    );
    assert_eq!(
        beta_set(&chat.values),
        expected_betas(&[
            "alpha",
            "beta",
            "effort-2025-11-24",
            "mcp-client-2025-04-04",
            "skills-2025-10-02",
            "code-execution-2025-08-25",
        ])
    );

    // The environment key answers when the request carries none, and its empty value counts as unset.
    case.api_key = None;
    case.environment.anthropic_aws_api_key = Some(SecretString::from("environment-key"));
    let chat = case.chat_headers(&records).expect("the environment key");
    assert_eq!(header(&chat.values, "x-api-key"), Some("environment-key"));
    case.environment.anthropic_aws_api_key = Some(SecretString::from(""));
    let chat = case
        .chat_headers(&records)
        .expect("an empty environment key resolves to none");
    assert!(header(&chat.values, "x-api-key").is_none());

    // A file block in the caller's messages turns on the file betas.
    let mut case = request_case();
    case.messages = json_list(json!([{
        "role": "user",
        "content": [{"type": "file", "file": {"file_id": "file-abc123"}}],
    }]));
    case.deployment.workspace_id = Some("workspace".to_owned());
    let chat = case
        .chat_headers(&records)
        .expect("the file block headers build");
    assert_eq!(
        beta_set(&chat.values),
        expected_betas(&["files-api-2025-04-14", "code-execution-2025-05-22"])
    );

    // The effort predicate also reads reasoning_effort, but only on an Opus 4.5 model.
    let mut case = request_case();
    case.model = "claude-opus-4-5";
    case.optional_params = json_map(json!({"reasoning_effort": "high"}));
    case.deployment.workspace_id = Some("workspace".to_owned());
    let chat = case
        .chat_headers(&records)
        .expect("the effort headers build");
    assert_eq!(
        beta_set(&chat.values),
        expected_betas(&["effort-2025-11-24"])
    );
}

// The native defaults, the x-api-key rule and B960's betas.
pub(crate) fn native_platform_headers() {
    let mut case = request_case();
    case.optional_params = json_map(json!({
        "compaction": {"type": "compaction"},
        "speed": "fast",
        "output_config": {"format": {"type": "json_schema"}},
    }));
    case.caller_headers = headers(json!({
        "anthropic-version": "2024-01-01",
        "content-type": "text/plain",
        "anthropic-beta": "alpha",
        "x-api-key": "",
    }));
    case.api_key = Some(SecretString::from("native-key"));
    case.api_base =
        Some("https://aws-external-anthropic.us-west-2.amazonaws.com/v1/messages".to_owned());
    case.deployment.workspace_id = Some("deployment-workspace".to_owned());
    let native = case.native_headers().expect("the native headers build");
    // should_filter_anthropic_beta_headers stays false, so the caller's alpha survives below.
    const { assert!(!FILTERS_ANTHROPIC_BETAS) };
    assert_eq!(
        header(&native.headers, "anthropic-version"),
        Some("2024-01-01")
    );
    assert_eq!(header(&native.headers, "content-type"), Some("text/plain"));
    // LiteLLM inserts the resolved key only when the exact header is absent, so an empty one stays.
    assert_eq!(header(&native.headers, "x-api-key"), Some(""));
    assert_eq!(
        header(&native.headers, WORKSPACE_HEADER),
        Some("deployment-workspace")
    );
    assert_eq!(
        native.api_base.as_deref(),
        Some("https://aws-external-anthropic.us-west-2.amazonaws.com/v1/messages")
    );
    assert_eq!(
        beta_set(&native.headers),
        expected_betas(&[
            "alpha",
            "compact-2026-01-12",
            "fast-mode-2026-02-01",
            "structured-outputs-2025-11-13",
        ])
    );

    // An absent key and an absent version or content type take their defaults.
    case.caller_headers = headers(json!({}));
    case.api_base = None;
    case.environment.anthropic_aws_api_key = Some(SecretString::from("environment-key"));
    let native = case
        .native_headers()
        .expect("the environment key is inserted");
    assert_eq!(
        header(&native.headers, "x-api-key"),
        Some("environment-key")
    );
    assert_eq!(
        header(&native.headers, "anthropic-version"),
        Some("2023-06-01")
    );
    assert_eq!(
        header(&native.headers, "content-type"),
        Some("application/json")
    );
    assert!(native.api_base.is_none());
    case.environment.anthropic_aws_api_key = Some(SecretString::from(""));
    let native = case
        .native_headers()
        .expect("an empty environment key inserts nothing");
    assert!(header(&native.headers, "x-api-key").is_none());
}

#[derive(Clone, Copy)]
pub(crate) enum BodyRoute {
    Chat,
    Native,
}

// B577 and B962 read the same request fields, so one function walks whichever body the case names.
pub(crate) fn platform_body(route: BodyRoute) {
    let records = Records::empty();
    match route {
        // B577 under provider bedrock with billing metadata stripped and unsupported params filtered.
        BodyRoute::Chat => {
            let mut case = request_case();
            case.messages = json_list(json!([
                {"role": "system", "content": [{"type": "text", "text": "x-anthropic-billing-header: 5f1a"}]},
                {"role": "system", "content": "You are a helpful assistant."},
                {"role": "user", "content": "hi"},
            ]));
            case.optional_params = json_map(json!({
                "max_tokens": 64,
                "workspace_id": "request-workspace",
                "context_management": {"edits": [{"type": "compact_20260112"}]},
            }));
            let defaults = defaults();
            let prepared = records
                .chat_body(&ChatRequest {
                    model: case.model,
                    messages: case.messages.clone(),
                    optional_params: case.optional_params.clone(),
                    litellm_params: case.litellm_params.clone(),
                    headers: AnthropicHeaders::default(),
                    defaults: &defaults,
                    catalog: records.catalog(),
                    default_chat_max_tokens: 4096,
                    modify_params: false,
                    global_drop_params: false,
                })
                .expect("the chat body prepares");
            let body = inspect_body(
                &prepared.prepared.body,
                "claude-sonnet-4-5",
                "\"max_tokens\":64",
            );
            let encoded = body.to_string();
            assert!(body.get("workspace_id").is_none(), "{encoded}");
            assert!(
                encoded.contains("You are a helpful assistant."),
                "{encoded}"
            );
            assert!(!encoded.contains("x-anthropic-billing-header"), "{encoded}");
            assert_eq!(
                prepared.warnings,
                vec![PlatformWarning::DroppedUnsupported {
                    params: vec!["context_management".to_owned()],
                }]
            );
            // The provider is bedrock, so B577 never adds the native compaction beta for the dropped edits.
            assert!(
                !prepared
                    .prepared
                    .headers
                    .values
                    .contains_key("anthropic-beta"),
                "{:?}",
                prepared.prepared.headers
            );
        }
        // B962 after one route strip, with the normal filter warnings and the billing block kept.
        BodyRoute::Native => {
            let messages = json_list(json!([
                {"role": "system", "content": [{"type": "text", "text": "You are a helpful assistant."}]},
                {"role": "system", "content": [{"type": "text", "text": "x-anthropic-billing-header: 5f1a"}]},
                {"role": "user", "content": "hi"},
            ]));
            let optional_params = json_map(json!({
                "max_tokens": 32,
                "claude_platform_unsupported_params": 5,
                "context_management": {"edits": []},
            }));
            let prompt_messages =
                convert::prompt_messages(&messages).expect("the case messages round trip");
            let prompt_params: IndexMap<String, PromptJson> =
                convert::mapped(&optional_params).expect("the case params round trip");
            let prepared = records
                .native_body(&NativeRequest {
                    model: "claude_platform/claude-sonnet-4-5",
                    messages: prompt_messages,
                    optional_params: prompt_params,
                    litellm_params: IndexMap::new(),
                })
                .expect("the native body prepares");
            let body = inspect_body(&prepared.body, "claude-sonnet-4-5", "\"max_tokens\":32");
            let encoded = body.to_string();
            assert!(
                body.get("claude_platform_unsupported_params").is_none(),
                "{encoded}"
            );
            assert!(
                encoded.contains("x-anthropic-billing-header: 5f1a"),
                "{encoded}"
            );
            assert_eq!(
                prepared.warnings,
                vec![
                    PlatformWarning::IgnoredOverride { type_name: "int" },
                    PlatformWarning::DroppedUnsupported {
                        params: vec!["context_management".to_owned()],
                    },
                ]
            );
        }
    }
}

fn anthropic_stream() -> Vec<TransportChunk> {
    let events = [
        (
            "message_start",
            r#"{"type":"message_start","message":{"id":"msg_1","role":"assistant","model":"claude-sonnet-4-5","usage":{"input_tokens":3,"output_tokens":0}}}"#,
        ),
        (
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        ),
        (
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"hello"}}"#,
        ),
        (
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
        ),
        (
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":5}}"#,
        ),
        ("message_stop", r#"{"type":"message_stop"}"#),
    ];
    let mut chunks = Vec::new();
    for (event, data) in events {
        chunks.push(TransportChunk::Text(format!("event: {event}")));
        chunks.push(TransportChunk::Text(format!("data: {data}")));
        chunks.push(TransportChunk::Text(String::new()));
    }
    chunks
}

// B583's Anthropic SSE iterator, with json_mode read as bool(json_mode).
pub(crate) fn native_anthropic_stream() {
    let ids = FixedIds::new(STREAM_ID);
    let recorded = stream_chunks(anthropic_stream(), Some(false), &ids);
    assert!(!recorded.is_empty(), "the stream produced no chunks");
    let expected_id = format!("chatcmpl-{STREAM_ID}");
    for value in &recorded {
        assert_eq!(value["id"].as_str(), Some(expected_id.as_str()), "{value}");
    }
    assert!(
        recorded
            .iter()
            .any(|value| value["content"].as_str() == Some("hello")),
        "{recorded:?}"
    );
    assert!(
        recorded
            .iter()
            .any(|value| value["model"].as_str() == Some("claude-sonnet-4-5")),
        "{recorded:?}"
    );
    assert!(
        recorded
            .iter()
            .any(|value| value["finish_reason"].is_string()),
        "{recorded:?}"
    );

    let plain = get_model_response_iterator(
        std::iter::empty::<Result<TransportChunk, ReceiveError<CounterError>>>(),
        StreamFlags {
            sync_stream: false,
            json_mode: None,
        },
        Counting,
        &ids,
        MAX_EVENT_BYTES,
    );
    assert_eq!(plain.converter.state.json_mode, Some(false));
    let json_mode = get_model_response_iterator(
        std::iter::empty::<Result<TransportChunk, ReceiveError<CounterError>>>(),
        StreamFlags {
            sync_stream: false,
            json_mode: Some(true),
        },
        Counting,
        &ids,
        MAX_EVENT_BYTES,
    );
    assert_eq!(json_mode.converter.state.json_mode, Some(true));
}

// Two configured identities never share a key or a workspace header.
pub(crate) fn credential_identity_isolation() {
    let records = Records::empty();
    let mut first = request_case();
    first.deployment.workspace_id = Some("first-workspace".to_owned());
    first.api_key = Some(SecretString::from("first-key"));
    let mut second = request_case();
    second.deployment.workspace_id = Some("second-workspace".to_owned());
    second.api_key = Some(SecretString::from("second-key"));

    let first_native = first.native_headers().expect("the first identity builds");
    let second_native = second.native_headers().expect("the second identity builds");
    assert_eq!(
        header(&first_native.headers, "x-api-key"),
        Some("first-key")
    );
    assert_eq!(
        header(&first_native.headers, WORKSPACE_HEADER),
        Some("first-workspace")
    );
    assert_eq!(
        header(&second_native.headers, "x-api-key"),
        Some("second-key")
    );
    assert_eq!(
        header(&second_native.headers, WORKSPACE_HEADER),
        Some("second-workspace")
    );
    assert!(
        !second_native
            .headers
            .values()
            .any(|value| value == "first-key")
    );
    assert!(
        !first_native
            .headers
            .values()
            .any(|value| value == "second-key")
    );

    let first_chat = first
        .chat_headers(&records)
        .expect("the first chat headers");
    let second_chat = second
        .chat_headers(&records)
        .expect("the second chat headers");
    assert_eq!(header(&first_chat.values, "x-api-key"), Some("first-key"));
    assert_eq!(header(&second_chat.values, "x-api-key"), Some("second-key"));
    assert_eq!(
        header(&first_chat.values, WORKSPACE_HEADER),
        Some("first-workspace")
    );
    assert_eq!(
        header(&second_chat.values, WORKSPACE_HEADER),
        Some("second-workspace")
    );
}

// The 401 path and the Debug of a prepared header map keep the secret out.
pub(crate) fn secret_safe_failure() {
    let records = Records::empty();
    let mut case = request_case();
    case.api_key = Some(SecretString::from("sk-secret-value"));
    let failure = case
        .chat_headers(&records)
        .expect_err("a missing workspace fails");
    let rendered = format!("{failure:?} {failure}");
    assert!(!rendered.contains("sk-secret-value"), "{rendered}");
    let failure = case
        .native_headers()
        .expect_err("a missing workspace fails for native too");
    let rendered = format!("{failure:?} {failure}");
    assert!(!rendered.contains("sk-secret-value"), "{rendered}");

    case.deployment.workspace_id = Some("workspace".to_owned());
    let chat = case.chat_headers(&records).expect("the headers build");
    assert_eq!(header(&chat.values, "x-api-key"), Some("sk-secret-value"));
    let rendered = format!("{chat:?}");
    assert!(!rendered.contains("sk-secret-value"), "{rendered}");
    let native = case.native_headers().expect("the native headers build");
    let rendered = format!("{native:?}");
    assert!(rendered.contains("x-api-key"), "{rendered}");
    assert!(!rendered.contains("sk-secret-value"), "{rendered}");
}

// A native transform failure maps to a status; the failed JSON round trip is the 500.
pub(crate) fn native_transform_status() {
    let error =
        serde_json::from_str::<serde_json::Value>("{").expect_err("the malformed JSON fails");
    let failure = NativeTransformError::Json(error);
    assert_eq!(failure.status_code(), Some(500));
}
