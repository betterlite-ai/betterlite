use std::collections::BTreeMap;
use std::time::SystemTime;

use betterlite_anthropic::anthropic_capability_catalog::DropParams;
use betterlite_config::sections::anthropic_capability_catalog::Settings;
use betterlite_logging::logging_pre_post::args::RawBody;
use betterlite_models::model_catalog_keys::ModelCostKeys;
use betterlite_models::model_metadata_resolution::dynamic::NoDynamicMetadata;
use betterlite_openai::compatible_wire::{LogInput, NEW_ID, ReplyOptions, system_clock};
use betterlite_prompts::arguments::json::{Json, Object};
use betterlite_prompts::bedrock_tools::name::ToolNameMap;
use indexmap::IndexMap;
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::support::{
    Answers, Call, FixedIds, Fixture, Logs, MODEL_COST, NOVA_PRO, NoReasoning, field, json,
    metadata_json, object, params, runtime, secret, secret_lookup, supplied, transform, value,
};
use crate::converse_options::supported::supported_openai_params as converse_supported;
use crate::converse_params::compose::ConverseParamsRequest;
use crate::converse_request::{CommonRequest, ConverseRequestBody};
use crate::converse_response::request::{OptionalParams, RawReply, ReplyRequest};
use crate::converse_response::transform::TransformCall;
use crate::image_call::plan::LogContext;
use crate::model_routes::ParamValue;
use crate::nova_chat::direct::{self, NovaChatArgs, NovaChatConfig, NovaEndpointRequest};
use crate::nova_chat::error::NovaChatError;
use crate::nova_chat::invoke::{self, InlineRequest};

const INLINE_SYSTEM: &str = r#"[
    {"text": "filler"},
    {"text": "kept", "cachePoint": {"type": "default"}},
    {"cachePoint": {"type": "default", "ttl": "1h"}},
    {"cachePoint": null},
    {"text": "tail"}
]"#;

const INLINE_MESSAGES: &str = r#"[
    {"role": "user", "name": "dropped", "content": [
        {"text": "first"},
        {"image": {"format": "png", "source": {"bytes": "AA=="}}},
        {"text": "second"},
        {"cachePoint": {"type": "default"}},
        {"cachePoint": {"type": "default", "ttl": "5m"}}
    ]},
    {"role": "assistant", "content": [
        {"text": "calling"},
        {"toolUse": {"toolUseId": "t1", "name": "lookup", "input": {}}},
        {"cachePoint": {"type": "default"}}
    ]},
    {"role": "user", "content": [{"cachePoint": {"type": "default"}}]},
    {"role": "assistant", "content": [{"text": "answer"}]}
]"#;

const INLINE_SYSTEM_EXPECTED: &str = r#"[
    {"text": "filler"},
    {"text": "kept", "cachePoint": {"type": "default", "ttl": "1h"}},
    {"cachePoint": null},
    {"text": "tail"}
]"#;

const INLINE_MESSAGES_EXPECTED: &str = r#"[
    {"role": "user", "content": [
        {"text": "first"},
        {"image": {"format": "png", "source": {"bytes": "AA=="}}},
        {"text": "second", "cachePoint": {"type": "default", "ttl": "5m"}}
    ]},
    {"role": "assistant", "content": [
        {"text": "calling", "cachePoint": {"type": "default"}},
        {"toolUse": {"toolUseId": "t1", "name": "lookup", "input": {}}}
    ]},
    {"role": "user", "content": []},
    {"role": "assistant", "content": [{"text": "answer"}]}
]"#;

const FILTER_OPTIONS: &str = r#"{
    "maxTokens": 64,
    "temperature": 0.2,
    "serviceTier": {"type": "priority"},
    "custom_field": {"a": 1}
}"#;

const SYSTEM_MESSAGES: &str =
    r#"[{"role": "system", "content": "be brief"}, {"role": "user", "content": "hi"}]"#;

const TOOL_OPTIONS: &str = r#"{
    "tools": [{"type": "function", "function": {
        "name": "lookup",
        "description": "finds things",
        "parameters": {"type": "object", "properties": {}}
    }}],
    "cache_control_injection_points": [{"location": "tool_config", "control": {"type": "ephemeral"}}]
}"#;

const MIXED_POINTS: &str = r#"{"cache_control_injection_points": [
    {"location": "tool_config", "control": {"type": "ephemeral"}},
    {"location": "system"}
]}"#;

const TOOL_CONFIG: &str = r#"{"tools": [{"toolSpec": {
    "name": "lookup",
    "description": "finds things",
    "inputSchema": {"json": {"type": "object", "properties": {}, "required": []}}
}}]}"#;

const CONVERSE_REPLY: &str = r#"{"output": {"message": {"role": "assistant", "content": [{"text": "done"}]}}, "stopReason": "end_turn", "usage": {"inputTokens": 10, "outputTokens": 5, "totalTokens": 15}}"#;

const CHAT_BODY: &str = r#"{
    "id": "chatcmpl-1",
    "object": "chat.completion",
    "created": 1700000000,
    "model": "previous-model",
    "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
    "usage": {"prompt_tokens": 1, "completion_tokens": 2, "total_tokens": 3}
}"#;

#[derive(Clone, Copy, Debug)]
enum Scenario {
    NovaInlineCachePoints,
    NovaFilterAllowedFields,
    NovaToolConfigInjection,
    DirectNovaEndpoint,
    NovaConverseDelegation,
    DirectNovaDefaults,
    DirectNovaResponseModel,
}

#[rstest]
#[case::nova_inline_cache_points(Scenario::NovaInlineCachePoints)]
#[case::nova_filter_allowed_fields(Scenario::NovaFilterAllowedFields)]
#[case::nova_tool_config_injection(Scenario::NovaToolConfigInjection)]
#[case::direct_nova_endpoint(Scenario::DirectNovaEndpoint)]
#[case::nova_converse_delegation(Scenario::NovaConverseDelegation)]
#[case::direct_nova_defaults(Scenario::DirectNovaDefaults)]
#[case::direct_nova_response_model(Scenario::DirectNovaResponseModel)]
fn nova_chat_cases(#[case] scenario: Scenario) {
    match scenario {
        Scenario::NovaInlineCachePoints => nova_inline_cache_points(),
        Scenario::NovaFilterAllowedFields => nova_filter_allowed_fields(),
        Scenario::NovaToolConfigInjection => nova_tool_config_injection(),
        Scenario::DirectNovaEndpoint => direct_nova_endpoint(),
        Scenario::NovaConverseDelegation => nova_converse_delegation(),
        Scenario::DirectNovaDefaults => direct_nova_defaults(),
        Scenario::DirectNovaResponseModel => direct_nova_response_model(),
    }
}

fn objects(source: &str) -> Vec<Object> {
    serde_json::from_str(source).expect("the object fixtures parse")
}

// A standalone point moves behind the last text block, overwriting one already there, and a point with no text before it is dropped with the extra message field.
fn nova_inline_cache_points() {
    let Json::List(system) = json(INLINE_SYSTEM) else {
        panic!("the system fixture is a list");
    };
    let Json::List(conversation) = json(INLINE_MESSAGES) else {
        panic!("the message fixture is a list");
    };
    let body = invoke::inline_cache_points(&InlineRequest {
        system: &system,
        messages: &conversation,
    })
    .expect("the blocks inline");
    assert_eq!(body.system, objects(INLINE_SYSTEM_EXPECTED));
    assert_eq!(body.messages, objects(INLINE_MESSAGES_EXPECTED));
    let missing_role = [json(r#"{"content": []}"#)];
    let role_error = invoke::inline_cache_points(&InlineRequest {
        system: &[],
        messages: &missing_role,
    })
    .expect_err("a message without a role is refused");
    assert!(matches!(
        &role_error,
        NovaChatError::BodyField { field: "role", .. }
    ));
    let text_content = [json(r#"{"role": "user", "content": "hi"}"#)];
    let content_error = invoke::inline_cache_points(&InlineRequest {
        system: &[],
        messages: &text_content,
    })
    .expect_err("a message content that is not a list is refused");
    assert!(matches!(
        &content_error,
        NovaChatError::BodyField {
            field: "content",
            ..
        }
    ));
    let text_system = [json(r#""not a block""#)];
    let system_error = invoke::inline_cache_points(&InlineRequest {
        system: &text_system,
        messages: &[],
    })
    .expect_err("a system block that is not an object is refused");
    assert!(matches!(
        &system_error,
        NovaChatError::BodyField {
            field: "system",
            ..
        }
    ));
    let not_a_message = [json("3")];
    let message_error = invoke::inline_cache_points(&InlineRequest {
        system: &[],
        messages: &not_a_message,
    })
    .expect_err("a message that is not an object is refused");
    assert!(matches!(
        &message_error,
        NovaChatError::BodyField {
            field: "messages",
            ..
        }
    ));
}

// /bedrock/invoke/ takes the five BedrockInvokeNovaRequest members, keeps the inference fields and drops an empty system.
fn nova_filter_allowed_fields() {
    let outcome = transform(&Call {
        messages: SYSTEM_MESSAGES,
        ..Call::new(NOVA_PRO, FILTER_OPTIONS)
    })
    .expect("the invoke body assembles");
    let body = value(&outcome.body);
    assert_eq!(
        field(&body, "inferenceConfig"),
        Some(&json(r#"{"maxTokens": 64, "temperature": 0.2}"#))
    );
    assert_eq!(
        field(&body, "system"),
        Some(&json(r#"[{"text": "be brief"}]"#))
    );
    let fields = body.as_object().expect("the body is an object");
    assert_eq!(fields.len(), 3);
    for allowed in ["inferenceConfig", "messages", "system"] {
        assert!(fields.contains_key(allowed));
    }
    let converse = ConverseRequestBody {
        messages: vec![json(r#"{"role": "user", "content": [{"text": "hi"}]}"#)],
        common: CommonRequest {
            inference_config: object(r#"{"maxTokens": 8}"#),
            additional_fields: None,
            response_paths: None,
            system: Vec::new(),
            guardrail_config: Some(json(r#"{"guardrailIdentifier": "g"}"#)),
            performance_config: None,
            service_tier: None,
            tool_config: Some(json(r#"{"tools": []}"#)),
            request_metadata: None,
            output_config: None,
        },
    };
    let converted = invoke::transform_converse(&converse).expect("the body converts");
    assert_eq!(
        value(&converted),
        json(
            r#"{"messages": [{"role": "user", "content": [{"text": "hi"}]}], "inferenceConfig": {"maxTokens": 8}, "toolConfig": {"tools": []}, "guardrailConfig": {"guardrailIdentifier": "g"}}"#
        )
    );
}

// A tool_config point is removed before the Converse transform, so the tool list never carries a cachePoint the Invoke API cannot cache.
fn nova_tool_config_injection() {
    let filtered = invoke::without_tool_config_points(&params(MIXED_POINTS));
    let kept = ParamValue::List(vec![ParamValue::Mapping(params(
        r#"{"location": "system"}"#,
    ))]);
    assert_eq!(filtered.get("cache_control_injection_points"), Some(&kept));
    let absent = params(r#"{"maxTokens": 8}"#);
    assert_eq!(invoke::without_tool_config_points(&absent), absent);
    let malformed =
        params(r#"{"cache_control_injection_points": [{"location": "tool_config"}, 3]}"#);
    assert_eq!(invoke::without_tool_config_points(&malformed), malformed);
    let only_tool = invoke::without_tool_config_points(&params(
        r#"{"cache_control_injection_points": [{"location": "tool_config"}]}"#,
    ));
    assert_eq!(
        only_tool.get("cache_control_injection_points"),
        Some(&ParamValue::List(Vec::new()))
    );
    let outcome = transform(&Call::new(NOVA_PRO, TOOL_OPTIONS)).expect("the invoke body assembles");
    let body = value(&outcome.body);
    assert_eq!(field(&body, "toolConfig"), Some(&json(TOOL_CONFIG)));
    assert!(field(&body, "system").is_none());
}

// api_base or AMAZON_NOVA_API_BASE or the default; api_key or the global key or AMAZON_NOVA_API_KEY or litellm.api_key, each empty value falling through.
fn direct_nova_endpoint() {
    let secrets = secret_lookup();
    let empty = BTreeMap::new();
    let defaulted = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: None,
            api_key: None,
            global_api_key: None,
            litellm_api_key: None,
        },
        &empty,
        &secrets,
    )
    .expect("the endpoint resolves");
    assert_eq!(defaulted.api_base, "https://api.nova.amazon.com/v1");
    assert!(defaulted.api_key.is_none());
    let environment = BTreeMap::from([
        (
            String::from("AMAZON_NOVA_API_BASE"),
            String::from("https://env.example/v1"),
        ),
        (String::from("AMAZON_NOVA_API_KEY"), String::from("env-key")),
    ]);
    let explicit_key = secret("explicit-key");
    let global_key = secret("global-key");
    let litellm_key = secret("litellm-key");
    let explicit = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: Some("https://explicit.example/v1"),
            api_key: Some(&explicit_key),
            global_api_key: Some(&global_key),
            litellm_api_key: Some(&litellm_key),
        },
        &environment,
        &secrets,
    )
    .expect("the endpoint resolves");
    assert_eq!(explicit.api_base, "https://explicit.example/v1");
    let explicit_choice = explicit.api_key.as_ref().expect("the explicit key wins");
    assert_eq!(explicit_choice.expose_secret(), "explicit-key");
    let blank = secret("");
    let from_environment = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: Some(""),
            api_key: Some(&blank),
            global_api_key: Some(&global_key),
            litellm_api_key: Some(&litellm_key),
        },
        &environment,
        &secrets,
    )
    .expect("the endpoint resolves");
    assert_eq!(from_environment.api_base, "https://env.example/v1");
    let global_choice = from_environment
        .api_key
        .as_ref()
        .expect("the global key beats the environment one");
    assert_eq!(global_choice.expose_secret(), "global-key");
    let from_litellm = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: None,
            api_key: None,
            global_api_key: None,
            litellm_api_key: Some(&litellm_key),
        },
        &empty,
        &secrets,
    )
    .expect("the endpoint resolves");
    let litellm_choice = from_litellm
        .api_key
        .as_ref()
        .expect("the litellm key is last");
    assert_eq!(litellm_choice.expose_secret(), "litellm-key");
    let empty_global = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: None,
            api_key: None,
            global_api_key: Some(&blank),
            litellm_api_key: Some(&litellm_key),
        },
        &empty,
        &secrets,
    )
    .expect("the endpoint resolves");
    let empty_global_choice = empty_global
        .api_key
        .as_ref()
        .expect("an empty global key falls through");
    assert_eq!(empty_global_choice.expose_secret(), "litellm-key");
    assert_eq!(direct::CUSTOM_LLM_PROVIDER.to_string(), "amazon_nova");
    // The chain is Python's `or`: a value never reached is never read, so a broken reference does not fail a call that supplies its own.
    let broken = BTreeMap::from([
        (
            String::from("AMAZON_NOVA_API_BASE"),
            String::from("oidc/circleci/nova"),
        ),
        (
            String::from("AMAZON_NOVA_API_KEY"),
            String::from("oidc/circleci/nova"),
        ),
    ]);
    let short_circuited = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: Some("https://explicit.example/v1"),
            api_key: Some(&explicit_key),
            global_api_key: None,
            litellm_api_key: None,
        },
        &broken,
        &secrets,
    )
    .expect("an explicit pair never reads the environment");
    assert_eq!(short_circuited.api_base, "https://explicit.example/v1");
    let short_circuit_key = short_circuited.api_key.as_ref().expect("the explicit key");
    assert_eq!(short_circuit_key.expose_secret(), "explicit-key");
    let global_short_circuit = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: Some("https://explicit.example/v1"),
            api_key: None,
            global_api_key: Some(&global_key),
            litellm_api_key: None,
        },
        &broken,
        &secrets,
    )
    .expect("a global key never reads the environment");
    let global_short_choice = global_short_circuit
        .api_key
        .as_ref()
        .expect("the global key");
    assert_eq!(global_short_choice.expose_secret(), "global-key");
    // The global key is a value: a reference-looking one is used as written, and the base keeps its trailing slash.
    let reference = secret("oidc/circleci/nova");
    let used_as_value = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: Some("https://explicit.example/v1/"),
            api_key: None,
            global_api_key: Some(&reference),
            litellm_api_key: None,
        },
        &empty,
        &secrets,
    )
    .expect("the global key is a value");
    assert_eq!(used_as_value.api_base, "https://explicit.example/v1/");
    assert_eq!(
        used_as_value
            .api_key
            .as_ref()
            .expect("the global key")
            .expose_secret(),
        "oidc/circleci/nova"
    );
    let blank_environment = BTreeMap::from([
        (String::from("AMAZON_NOVA_API_BASE"), String::new()),
        (String::from("AMAZON_NOVA_API_KEY"), String::new()),
    ]);
    let from_blank_environment = direct::resolve_endpoint(
        NovaEndpointRequest {
            api_base: None,
            api_key: None,
            global_api_key: None,
            litellm_api_key: None,
        },
        &blank_environment,
        &secrets,
    )
    .expect("an empty variable is unset");
    assert_eq!(
        from_blank_environment.api_base,
        "https://api.nova.amazon.com/v1"
    );
    assert!(from_blank_environment.api_key.is_none());
}

fn nova_converse_delegation() {
    converse_supported_params();
    converse_params_mapping();
    converse_reply_delegation();
}

// The Nova invoke config adds no parameter: its list is the Converse one, web search included.
fn converse_supported_params() {
    let model_cost = params(MODEL_COST);
    let fixture = Fixture::load(&model_cost);
    let catalogs = fixture.catalogs();
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let resolution = fixture.resolution();
    let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
    let supported = runtime().block_on(converse_supported(NOVA_PRO, &mut environment, &catalogs));
    assert!(supported.iter().any(|name| name == "web_search_options"));
    assert!(!supported.iter().any(|name| name == "metadata"));
    assert!(!supported.iter().any(|name| name == "top_k"));
}

// max_completion_tokens becomes maxTokens in the caller's own map, through the Converse mapping.
fn converse_params_mapping() {
    let model_cost = params(MODEL_COST);
    let fixture = Fixture::load(&model_cost);
    let catalogs = fixture.catalogs();
    let answers = Answers(BTreeMap::new());
    let mut keys = ModelCostKeys::default();
    let dynamic = NoDynamicMetadata;
    let resolution = fixture.resolution();
    let mut environment = fixture.environment(&resolution, &mut keys, &dynamic);
    let budgets = Settings::default();
    let non_default = params(r#"{"max_completion_tokens": 32}"#);
    let mut mapped = params(r#"{"maxTokens": 10}"#);
    let mut names = ToolNameMap::new();
    runtime()
        .block_on(invoke::map_openai_params(
            &ConverseParamsRequest {
                model: NOVA_PRO,
                drop: DropParams {
                    global: false,
                    call: false,
                },
                non_default_params: &non_default,
                catalogs: &catalogs,
                budgets: &budgets,
                lookup: &answers,
                packaged: fixture.packaged(),
            },
            &mut mapped,
            &mut environment,
            &mut names,
        ))
        .expect("the options map");
    assert_eq!(mapped, params(r#"{"maxTokens": 32}"#));
}

// The reply conversion is the Converse one: a Converse body becomes the same ModelResponse.
fn converse_reply_delegation() {
    let logs = Logs::new();
    let Logs { mut state, parts } = logs;
    let env = parts.env();
    let messages = metadata_json(r#"[{"role": "user", "content": "hi"}]"#);
    let data: RawBody = serde_json::from_str(r#"{"model": "amazon.nova-pro-v1:0"}"#)
        .expect("the request data parses");
    let optional = OptionalParams::default();
    let mappings: IndexMap<String, String> = IndexMap::new();
    let headers: Vec<(String, String)> = Vec::new();
    let ids = FixedIds;
    let counter = NoReasoning;
    let reply = invoke::transform_response(
        TransformCall {
            request: ReplyRequest {
                model: NOVA_PRO,
                messages: &messages,
                data,
                json_mode: None,
                optional: &optional,
                tool_name_mappings: &mappings,
            },
            log: LogContext {
                state: &mut state,
                env: &env,
            },
            now: SystemTime::UNIX_EPOCH,
        },
        &RawReply {
            headers: &headers,
            body: CONVERSE_REPLY.as_bytes(),
        },
        supplied("chatcmpl-supplied"),
        &ids,
        &counter,
    )
    .expect("the reply converts");
    assert_eq!(reply.choices[0].message.content.as_deref(), Some("done"));
    assert_eq!(
        reply.usage.as_ref().map(|usage| usage.total_tokens),
        Some(15)
    );
}

// get_config reads the class dict: the six constructor fields, with metadata only when it was set.
fn direct_nova_defaults() {
    let unset = NovaChatConfig::default();
    assert!(unset.config().expect("the config renders").is_empty());
    let configured = NovaChatConfig {
        args: NovaChatArgs {
            max_completion_tokens: Some(64),
            max_tokens: Some(32),
            temperature: Some(1),
            top_p: Some(0),
            tools: Some(vec![json(r#"{"type": "function"}"#)]),
            reasoning_effort: Some(vec![json(r#""low""#)]),
        },
        metadata: None,
    };
    let rendered = value(&configured.config().expect("the config renders"));
    assert_eq!(field(&rendered, "max_completion_tokens"), Some(&json("64")));
    assert_eq!(field(&rendered, "max_tokens"), Some(&json("32")));
    assert_eq!(field(&rendered, "temperature"), Some(&json("1")));
    assert_eq!(field(&rendered, "top_p"), Some(&json("0")));
    assert_eq!(
        field(&rendered, "tools"),
        Some(&json(r#"[{"type": "function"}]"#))
    );
    assert_eq!(
        field(&rendered, "reasoning_effort"),
        Some(&json(r#"["low"]"#))
    );
    assert!(field(&rendered, "metadata").is_none());
    let with_metadata = NovaChatConfig {
        args: NovaChatArgs::default(),
        metadata: Some(7),
    };
    let configured_metadata = value(&with_metadata.config().expect("the config renders"));
    assert_eq!(field(&configured_metadata, "metadata"), Some(&json("7")));
    assert_eq!(
        direct::SUPPORTED_OPENAI_PARAMS,
        [
            "top_p",
            "temperature",
            "max_tokens",
            "max_completion_tokens",
            "metadata",
            "stop",
            "stream",
            "stream_options",
            "tools",
            "tool_choice",
            "reasoning_effort",
        ]
    );
}

// The reply names amazon-nova/<model> so the cost lookup finds the direct provider's price.
fn direct_nova_response_model() {
    let logs = Logs::new();
    let Logs { mut state, parts } = logs;
    let env = parts.env();
    let options = ReplyOptions {
        json_mode: false,
        provider: None,
        base_model: None,
        identifier: NEW_ID,
        clock: system_clock,
    };
    let reply = direct::transform_response(
        NOVA_PRO,
        CHAT_BODY.as_bytes(),
        &options,
        &mut state,
        &env,
        LogInput {
            messages: metadata_json(r#"[{"role": "user", "content": "hi"}]"#),
            data: RawBody::default(),
        },
    )
    .expect("the reply converts");
    assert_eq!(reply.response.model, format!("amazon-nova/{NOVA_PRO}"));
    assert_eq!(
        reply.response.choices[0].message.content.as_deref(),
        Some("hi")
    );
    assert_eq!(reply.response.usage.total_tokens, 3);
}
