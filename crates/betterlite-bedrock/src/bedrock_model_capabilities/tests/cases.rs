use betterlite_models::catalog::{ModelCatalog, UnknownFields};

use super::super::CapabilityError;
use super::super::catalog_flags::{
    accepts_cache_points, is_claude_4_5_on_bedrock, supports_parallel_tools,
};
use super::super::effort::normalize_opus_effort;
use super::super::endpoints::supports_openai_responses;
use super::super::mantle::mantle_messages_url;
use super::super::strict_tools::converse_supports_strict_tools;
use super::support::{self, Answers, map, runtime};
use crate::model_routes::ParamValue;

fn outcome(result: Result<bool, CapabilityError>) -> &'static str {
    match result {
        Ok(true) => "true",
        Ok(false) => "false",
        Err(CapabilityError::MalformedEntry { .. }) => "malformed",
        Err(CapabilityError::NotIterable { .. }) => "not-iterable",
        Err(CapabilityError::UnhashableEffort) => "unhashable",
        Err(CapabilityError::Pattern(_)) => "pattern",
    }
}

fn value(source: &str) -> ParamValue {
    serde_json::from_str(source).expect("fixture value parses")
}

pub(super) fn mantle_api_base() {
    let region = "eu-west-1";
    let default = "https://bedrock-mantle.eu-west-1.api.aws/anthropic/v1/messages";
    assert_eq!(mantle_messages_url(None, None, None, region), default);
    assert_eq!(mantle_messages_url(None, None, Some(""), region), default);
    assert_eq!(
        mantle_messages_url(None, None, Some("/v1"), region),
        default,
        "an environment base that strips to nothing falls back to the regional host"
    );
    assert_eq!(mantle_messages_url(None, None, Some("/"), region), default);
    assert_eq!(
        mantle_messages_url(None, None, Some("https://env.example"), region),
        "https://env.example/anthropic/v1/messages"
    );
    assert_eq!(
        mantle_messages_url(None, None, Some("https://env.example/"), region),
        "https://env.example/anthropic/v1/messages"
    );
    assert_eq!(
        mantle_messages_url(None, None, Some("https://env.example/v1"), region),
        "https://env.example/anthropic/v1/messages",
        "the environment base drops a trailing /v1"
    );
    assert_eq!(
        mantle_messages_url(None, None, Some("https://env.example/openai/v1"), region),
        "https://env.example/anthropic/v1/messages",
        "the longer OpenAI suffix is stripped before /v1"
    );
    assert_eq!(
        mantle_messages_url(None, None, Some("https://env.example/openai/v1/"), region),
        "https://env.example/anthropic/v1/messages"
    );
    assert_eq!(
        mantle_messages_url(
            None,
            None,
            Some("https://env.example/anthropic/v1/messages"),
            region
        ),
        "https://env.example/anthropic/v1/messages"
    );
    assert_eq!(
        mantle_messages_url(Some("https://explicit.example/v1"), None, None, region),
        "https://explicit.example/v1/anthropic/v1/messages",
        "an explicit base keeps its /v1"
    );
    assert_eq!(
        mantle_messages_url(
            Some("https://explicit.example/openai/v1"),
            None,
            None,
            region
        ),
        "https://explicit.example/openai/v1/anthropic/v1/messages",
        "an explicit base keeps its /openai/v1"
    );
    assert_eq!(
        mantle_messages_url(
            Some("https://api.example"),
            Some("https://runtime.example"),
            Some("https://env.example"),
            region
        ),
        "https://api.example/anthropic/v1/messages",
        "api_base wins over both fallbacks"
    );
    assert_eq!(
        mantle_messages_url(
            Some(""),
            Some("https://runtime.example"),
            Some("https://env.example"),
            region
        ),
        "https://runtime.example/anthropic/v1/messages",
        "an empty api_base is skipped"
    );
    assert_eq!(
        mantle_messages_url(
            None,
            Some("https://runtime.example/v1"),
            Some("https://env.example"),
            region
        ),
        "https://runtime.example/v1/anthropic/v1/messages"
    );
    assert_eq!(
        mantle_messages_url(
            Some("https://a.example/anthropic/v1/messages"),
            None,
            None,
            region
        ),
        "https://a.example/anthropic/v1/messages"
    );
    assert_eq!(
        mantle_messages_url(
            Some("https://a.example/anthropic/v1/messages/"),
            None,
            None,
            region
        ),
        "https://a.example/anthropic/v1/messages"
    );
    assert_eq!(
        mantle_messages_url(Some("/"), None, None, region),
        "/anthropic/v1/messages"
    );
    assert_eq!(
        mantle_messages_url(None, None, None, "cn-north-1"),
        "https://bedrock-mantle.cn-north-1.api.aws/anthropic/v1/messages",
        "the regional host carries no China-domain rewrite"
    );
}

pub(super) fn native_responses() {
    let cost = map(r#"{
            "listed": {"supported_endpoints": ["/v1/chat/completions", "/v1/responses"]},
            "string": {"supported_endpoints": "/v1/responses"},
            "substring": {"supported_endpoints": "/prefix/v1/responses/suffix"},
            "absent": {},
            "empty": {"supported_endpoints": []},
            "null": {"supported_endpoints": null},
            "zero": {"supported_endpoints": 0},
            "false": {"supported_endpoints": false},
            "object": {"supported_endpoints": {"/v1/responses": 1}},
            "object_miss": {"supported_endpoints": {"other": 1}},
            "number": {"supported_endpoints": 5},
            "flag": {"supported_endpoints": true},
            "not_object": 5,
            "bedrock/prefixed": {"supported_endpoints": ["/v1/responses"]},
            "bedrock/only_the_pair": {"supported_endpoints": "/v1/responses"}
        }"#);
    assert_eq!(
        outcome(supports_openai_responses(Some("listed"), &cost)),
        "true"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("string"), &cost)),
        "true"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("substring"), &cost)),
        "true",
        "a string endpoint list is a substring test"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("absent"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("empty"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("null"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("zero"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("false"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("object"), &cost)),
        "true"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("object_miss"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("number"), &cost)),
        "not-iterable"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("flag"), &cost)),
        "not-iterable"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("not_object"), &cost)),
        "false",
        "a non-object entry is skipped, not reported"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("prefixed"), &cost)),
        "true",
        "the bedrock/-prefixed key is inspected after the raw one"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("only_the_pair"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_openai_responses(Some("missing"), &cost)),
        "false"
    );
    assert_eq!(outcome(supports_openai_responses(None, &cost)), "false");
    assert_eq!(outcome(supports_openai_responses(Some(""), &cost)), "false");
}

pub(super) fn cache_point_unknown_model() {
    let cost = map(r#"{
            "openai.gpt-5": {"supports_prompt_caching": true},
            "us.openai.gpt-5": {"supports_prompt_caching": true},
            "myopenai.gpt-5": {"supports_prompt_caching": true},
            "cached": {"supports_prompt_caching": true},
            "uncached": {"supports_prompt_caching": false},
            "flag_one": {"supports_prompt_caching": 1},
            "empty": {},
            "null_entry": null,
            "zero_entry": 0,
            "malformed": 5,
            "anthropic.claude-regional-v1:0": {"supports_prompt_caching": true},
            "us.anthropic.claude-short": {"supports_prompt_caching": true},
            "anthropic.claude-short": 5,
            "us.anthropic.claude-bad": 5,
            "anthropic.claude-bad": {"supports_prompt_caching": true}
        }"#);
    assert_eq!(outcome(accepts_cache_points(Some("cached"), &cost)), "true");
    assert_eq!(
        outcome(accepts_cache_points(Some("uncached"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(accepts_cache_points(Some("flag_one"), &cost)),
        "false",
        "an integer one is not true"
    );
    assert_eq!(outcome(accepts_cache_points(Some("empty"), &cost)), "false");
    assert_eq!(
        outcome(accepts_cache_points(Some("null_entry"), &cost)),
        "true",
        "a null value is not an entry"
    );
    assert_eq!(
        outcome(accepts_cache_points(Some("zero_entry"), &cost)),
        "malformed"
    );
    assert_eq!(
        outcome(accepts_cache_points(Some("malformed"), &cost)),
        "malformed"
    );
    assert_eq!(
        outcome(accepts_cache_points(Some("openai.gpt-5"), &cost)),
        "false",
        "the OpenAI family never accepts explicit cache points"
    );
    assert_eq!(
        outcome(accepts_cache_points(Some("us.openai.gpt-5"), &cost)),
        "false"
    );
    assert_eq!(
        outcome(accepts_cache_points(Some("myopenai.gpt-5"), &cost)),
        "true",
        "the family pattern needs a leading dot, slash or start"
    );
    assert_eq!(
        outcome(accepts_cache_points(
            Some("us.anthropic.claude-regional-v1:0"),
            &cost
        )),
        "true",
        "the base model entry answers when the raw name has none"
    );
    assert_eq!(
        outcome(accepts_cache_points(
            Some("us.anthropic.claude-short"),
            &cost
        )),
        "true",
        "a true entry short-circuits before a malformed base entry"
    );
    assert_eq!(
        outcome(accepts_cache_points(Some("us.anthropic.claude-bad"), &cost)),
        "malformed",
        "a malformed raw entry fails before a true base entry"
    );
    assert_eq!(
        outcome(accepts_cache_points(Some("missing"), &cost)),
        "true"
    );
    assert_eq!(outcome(accepts_cache_points(None, &cost)), "true");
    assert_eq!(
        outcome(accepts_cache_points(Some(""), &cost)),
        "true",
        "an empty name is not the missing model"
    );
}

pub(super) fn strict_tools_fallback() {
    let packaged = ModelCatalog::parse(
        br#"{
            "strict-false": {"bedrock_converse_supports_strict_tools": false},
            "strict-true": {"bedrock_converse_supports_strict_tools": true}
        }"#,
        UnknownFields::Warn,
    )
    .expect("the packaged fixture parses");
    let answers = Answers::new(&[
        ("anthropic.claude-by-key", r#"{"key": "strict-false"}"#),
        ("anthropic.claude-by-true-key", r#"{"key": "strict-true"}"#),
        (
            "anthropic.claude-flag-true",
            r#"{"bedrock_converse_supports_strict_tools": true}"#,
        ),
        (
            "anthropic.claude-flag-false",
            r#"{"bedrock_converse_supports_strict_tools": false}"#,
        ),
        (
            "anthropic.claude-flag-null",
            r#"{"bedrock_converse_supports_strict_tools": null, "key": "strict-false"}"#,
        ),
        (
            "anthropic.claude-no-key",
            r#"{"bedrock_converse_supports_strict_tools": null}"#,
        ),
        (
            "anthropic.claude-v1",
            r#"{"bedrock_converse_supports_strict_tools": true}"#,
        ),
        (
            "anthropic.claude",
            r#"{"bedrock_converse_supports_strict_tools": false}"#,
        ),
        ("anthropic.claude-gap-v1:0", r#"{"mode": "chat"}"#),
        (
            "anthropic.claude-gap",
            r#"{"bedrock_converse_supports_strict_tools": false}"#,
        ),
        ("anthropic.claude-nokey-v1:0", r#"{"key": "no-such-entry"}"#),
        (
            "anthropic.claude-nokey",
            r#"{"bedrock_converse_supports_strict_tools": false}"#,
        ),
    ]);
    let runtime = runtime();
    let ask =
        |model: &str| runtime.block_on(converse_supports_strict_tools(model, &answers, &packaged));
    assert!(
        !ask("anthropic.claude-by-key"),
        "a string key selects the packaged entry and its false flag"
    );
    assert!(ask("anthropic.claude-by-true-key"));
    assert!(ask("anthropic.claude-flag-true"));
    assert!(!ask("anthropic.claude-flag-false"));
    assert!(
        !ask("anthropic.claude-flag-null"),
        "a null flag is not a boolean and falls through to the key"
    );
    assert!(
        ask("anthropic.claude-no-key"),
        "a lookup with neither a boolean nor a key leaves the default on"
    );
    assert!(
        ask("anthropic.claude-v1"),
        "the base name itself answers before a stripped variant"
    );
    assert!(
        !ask("anthropic.claude-by-key-v1:0"),
        "the version-stripped candidate answers when the raw name is unmapped"
    );
    assert!(
        !ask("amazon.nova-pro-v1:0"),
        "a non-Anthropic family never forwards strict tools"
    );
    assert!(
        ask("anthropic.claude-unmapped"),
        "every suppressed lookup failure leaves the default on"
    );
    assert!(
        ask("anthropic.claude-vnext"),
        "a -v that does not start a numeric version stays in the name"
    );
    assert!(
        ask("anthropic.claude-v1:beta"),
        "a non-numeric revision stays in the name"
    );
    assert!(
        !ask("anthropic.claude-gap-v1:0"),
        "a candidate without a flag or key moves on to the stripped name"
    );
    assert!(
        !ask("anthropic.claude-nokey-v1:0"),
        "a key with no packaged entry moves on to the stripped name"
    );
    assert!(
        !ask("anthropic.claude-gap-v١"),
        "a non-ASCII decimal digit is a version separator too"
    );
    assert!(
        !ask("anthropic.claude-gap-v1:١"),
        "a non-ASCII decimal digit is a revision separator too"
    );
}

pub(super) fn effort_ceiling() {
    let packaged = ModelCatalog::parse(
        br#"{
            "ceiling-low": {"bedrock_output_config_effort_ceiling": "low"},
            "ceiling-medium": {"bedrock_output_config_effort_ceiling": "medium"},
            "ceiling-high": {"bedrock_output_config_effort_ceiling": "high"},
            "ceiling-max": {"bedrock_output_config_effort_ceiling": "max"},
            "ceiling-xhigh": {"bedrock_output_config_effort_ceiling": "xhigh"},
            "ceiling-minimal": {"bedrock_output_config_effort_ceiling": "minimal"},
            "ceiling-none": {"bedrock_output_config_effort_ceiling": "none"}
        }"#,
        UnknownFields::Warn,
    )
    .expect("the packaged fixture parses");
    let answers = Answers::new(&[
        ("opus", r#"{"key": "ceiling-high"}"#),
        ("opus-low", r#"{"key": "ceiling-low"}"#),
        ("opus-medium", r#"{"key": "ceiling-medium"}"#),
        (
            "opus-max",
            r#"{"bedrock_output_config_effort_ceiling": "max"}"#,
        ),
        ("opus-xhigh", r#"{"key": "ceiling-xhigh"}"#),
        (
            "opus-unknown",
            r#"{"bedrock_output_config_effort_ceiling": "ultra"}"#,
        ),
        ("opus-minimal", r#"{"key": "ceiling-minimal"}"#),
        ("opus-none", r#"{"key": "ceiling-none"}"#),
        ("opus-bare", r#"{"key": "missing-entry"}"#),
    ]);
    let runtime = runtime();
    let normalize = |model: &str, config: &str| {
        let mut output_config = value(config);
        let result = runtime.block_on(normalize_opus_effort(
            model,
            &mut output_config,
            &answers,
            &packaged,
        ));
        (result, output_config)
    };
    let clamped = |model: &str, config: &str, expected: &str| {
        let (result, output_config) = normalize(model, config);
        assert!(result.is_ok(), "{model} {config} failed to normalize");
        assert_eq!(output_config, value(expected), "{model} {config}");
    };
    clamped(
        "opus",
        r#"{"effort": "xhigh", "format": "x"}"#,
        r#"{"effort": "high", "format": "x"}"#,
    );
    clamped("opus", r#"{"effort": "low"}"#, r#"{"effort": "low"}"#);
    clamped("opus", r#"{"effort": "high"}"#, r#"{"effort": "high"}"#);
    clamped("opus-low", r#"{"effort": "xhigh"}"#, r#"{"effort": "low"}"#);
    clamped(
        "opus-medium",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "medium"}"#,
    );
    clamped("opus-max", r#"{"effort": "xhigh"}"#, r#"{"effort": "max"}"#);
    clamped("opus-max", r#"{"effort": "max"}"#, r#"{"effort": "max"}"#);
    clamped(
        "opus-xhigh",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    );
    clamped("opus", r#"{"effort": "ultra"}"#, r#"{"effort": "ultra"}"#);
    clamped("opus", r#"{"effort": null}"#, r#"{"effort": null}"#);
    clamped("opus", r#"{"effort": 5}"#, r#"{"effort": 5}"#);
    clamped("opus", r#"{"effort": true}"#, r#"{"effort": true}"#);
    clamped("opus", r#"{}"#, r#"{}"#);
    clamped("opus", r#""plain""#, r#""plain""#);
    clamped(
        "opus-unknown",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    );
    clamped(
        "opus-minimal",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    );
    clamped(
        "opus-none",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    );
    clamped(
        "opus-bare",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    );
    clamped(
        "unmapped",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    );
    assert!(matches!(
        normalize("opus", r#"{"effort": ["high"]}"#).0,
        Err(CapabilityError::UnhashableEffort)
    ));
    assert!(matches!(
        normalize("opus", r#"{"effort": {"value": "high"}}"#).0,
        Err(CapabilityError::UnhashableEffort)
    ));
}

pub(super) fn parallel_tools() {
    let cost = map(r#"{
            "one": {"supports_parallel_tool_use_config": true},
            "integer": {"supports_parallel_tool_use_config": 1},
            "off": {"supports_parallel_tool_use_config": false},
            "empty": {},
            "null_entry": null,
            "false_entry": false,
            "zero_entry": 0,
            "malformed": 5,
            "text_entry": "",
            "anthropic.claude-p-v1:0": {"supports_parallel_tool_use_config": true},
            "us.anthropic.claude-q-v1:0": {"supports_parallel_tool_use_config": true},
            "anthropic.claude-q-v1:0": 5
        }"#);
    assert_eq!(outcome(supports_parallel_tools("one", &cost)), "true");
    assert_eq!(
        outcome(supports_parallel_tools("integer", &cost)),
        "false",
        "an integer one is not a boolean true"
    );
    assert_eq!(outcome(supports_parallel_tools("off", &cost)), "false");
    assert_eq!(outcome(supports_parallel_tools("empty", &cost)), "false");
    assert_eq!(
        outcome(supports_parallel_tools("null_entry", &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_parallel_tools("false_entry", &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_parallel_tools("zero_entry", &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_parallel_tools("malformed", &cost)),
        "malformed"
    );
    assert_eq!(
        outcome(supports_parallel_tools("text_entry", &cost)),
        "false"
    );
    assert_eq!(
        outcome(supports_parallel_tools("us.anthropic.claude-p-v1:0", &cost)),
        "true",
        "the base model entry enables it"
    );
    assert_eq!(
        outcome(supports_parallel_tools("us.anthropic.claude-q-v1:0", &cost)),
        "true",
        "a true entry short-circuits before a malformed base entry"
    );
    assert_eq!(outcome(supports_parallel_tools("missing", &cost)), "false");
}

pub(super) fn extended_cache_ttl() {
    let cost = map(r#"{
            "priced": {"cache_creation_input_token_cost_above_1hr": 0.000006},
            "zero": {"cache_creation_input_token_cost_above_1hr": 0},
            "false_flag": {"cache_creation_input_token_cost_above_1hr": false},
            "null_flag": {"cache_creation_input_token_cost_above_1hr": null},
            "absent": {},
            "null_entry": null,
            "malformed": 5,
            "anthropic.claude-e-v1:0": {"cache_creation_input_token_cost_above_1hr": 1}
        }"#);
    assert_eq!(outcome(is_claude_4_5_on_bedrock("priced", &cost)), "true");
    assert_eq!(
        outcome(is_claude_4_5_on_bedrock("zero", &cost)),
        "true",
        "a zero one-hour price still declares the field"
    );
    assert_eq!(
        outcome(is_claude_4_5_on_bedrock("false_flag", &cost)),
        "true",
        "a false one-hour price still declares the field"
    );
    assert_eq!(
        outcome(is_claude_4_5_on_bedrock("null_flag", &cost)),
        "false"
    );
    assert_eq!(outcome(is_claude_4_5_on_bedrock("absent", &cost)), "false");
    assert_eq!(
        outcome(is_claude_4_5_on_bedrock("null_entry", &cost)),
        "false"
    );
    assert_eq!(
        outcome(is_claude_4_5_on_bedrock("malformed", &cost)),
        "malformed"
    );
    assert_eq!(
        outcome(is_claude_4_5_on_bedrock(
            "us.anthropic.claude-e-v1:0",
            &cost
        )),
        "true",
        "the base model entry declares it"
    );
    assert_eq!(outcome(is_claude_4_5_on_bedrock("missing", &cost)), "false");
}

pub(super) fn tool_search() {
    let cost = r#"{
        "bedrock/claude-sonnet-4-5": {"litellm_provider": "bedrock", "supports_tool_search": false},
        "bedrock/fixture-tool-search": {"litellm_provider": "bedrock", "supports_tool_search": true},
        "bedrock/fixture-no-flag": {"litellm_provider": "bedrock", "mode": "chat"}
    }"#;
    let answers = support::tool_search_answers(
        cost,
        &[
            "claude-sonnet-4-5",
            "fixture-tool-search",
            "fixture-no-flag",
            "claude-sonnet-4-5-20990101",
            "mystery-model",
        ],
    );
    assert_eq!(
        answers,
        vec![false, true, false, true, false],
        "an explicit false wins, the catalog flag answers, and the Claude rule fills an unmapped id"
    );
}
