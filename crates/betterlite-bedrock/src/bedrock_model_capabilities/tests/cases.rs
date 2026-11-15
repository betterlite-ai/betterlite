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

const MANTLE_REGION: &str = "eu-west-1";
const MANTLE_DEFAULT_URL: &str = "https://bedrock-mantle.eu-west-1.api.aws/anthropic/v1/messages";
const MANTLE_ENVIRONMENT_URL: &str = "https://env.example/anthropic/v1/messages";

const MANTLE_CASES: &[(Option<&str>, Option<&str>, Option<&str>, &str, &str)] = &[
    (None, None, None, MANTLE_REGION, MANTLE_DEFAULT_URL),
    (None, None, Some(""), MANTLE_REGION, MANTLE_DEFAULT_URL),
    (None, None, Some("/v1"), MANTLE_REGION, MANTLE_DEFAULT_URL),
    (None, None, Some("/"), MANTLE_REGION, MANTLE_DEFAULT_URL),
    (
        None,
        None,
        Some("https://env.example"),
        MANTLE_REGION,
        MANTLE_ENVIRONMENT_URL,
    ),
    (
        None,
        None,
        Some("https://env.example/"),
        MANTLE_REGION,
        MANTLE_ENVIRONMENT_URL,
    ),
    (
        None,
        None,
        Some("https://env.example/v1"),
        MANTLE_REGION,
        MANTLE_ENVIRONMENT_URL,
    ),
    (
        None,
        None,
        Some("https://env.example/openai/v1"),
        MANTLE_REGION,
        MANTLE_ENVIRONMENT_URL,
    ),
    (
        None,
        None,
        Some("https://env.example/openai/v1/"),
        MANTLE_REGION,
        MANTLE_ENVIRONMENT_URL,
    ),
    (
        None,
        None,
        Some("https://env.example/anthropic/v1/messages"),
        MANTLE_REGION,
        MANTLE_ENVIRONMENT_URL,
    ),
    (
        Some("https://explicit.example/v1"),
        None,
        None,
        MANTLE_REGION,
        "https://explicit.example/v1/anthropic/v1/messages",
    ),
    (
        Some("https://explicit.example/openai/v1"),
        None,
        None,
        MANTLE_REGION,
        "https://explicit.example/openai/v1/anthropic/v1/messages",
    ),
    (
        Some("https://api.example"),
        Some("https://runtime.example"),
        Some("https://env.example"),
        MANTLE_REGION,
        "https://api.example/anthropic/v1/messages",
    ),
    (
        Some(""),
        Some("https://runtime.example"),
        Some("https://env.example"),
        MANTLE_REGION,
        "https://runtime.example/anthropic/v1/messages",
    ),
    (
        None,
        Some("https://runtime.example/v1"),
        Some("https://env.example"),
        MANTLE_REGION,
        "https://runtime.example/v1/anthropic/v1/messages",
    ),
    (
        Some("https://a.example/anthropic/v1/messages"),
        None,
        None,
        MANTLE_REGION,
        "https://a.example/anthropic/v1/messages",
    ),
    (
        Some("https://a.example/anthropic/v1/messages/"),
        None,
        None,
        MANTLE_REGION,
        "https://a.example/anthropic/v1/messages",
    ),
    (
        Some("/"),
        None,
        None,
        MANTLE_REGION,
        "/anthropic/v1/messages",
    ),
    (
        None,
        None,
        None,
        "cn-north-1",
        "https://bedrock-mantle.cn-north-1.api.aws/anthropic/v1/messages",
    ),
];

const NATIVE_RESPONSES_MAP: &str = r#"{
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
            "chat_only_string": {"supported_endpoints": "/v1/chat/completions"},
            "chat_only_list": {"supported_endpoints": ["/v1/chat/completions"]},
            "raising": {"supported_endpoints": true},
            "bedrock/raising": {"supported_endpoints": ["/v1/responses"]},
            "anthropic.claude-alias-v1:0": {"supported_endpoints": ["/v1/responses"]},
            "bedrock/prefixed": {"supported_endpoints": ["/v1/responses"]},
            "bedrock/only_the_pair": {"supported_endpoints": "/v1/responses"}
        }"#;

const NATIVE_RESPONSES_MODELS: &[(&str, &str, &str)] = &[
    (
        "listed",
        "true",
        "a list that offers the Responses endpoint matches",
    ),
    (
        "string",
        "true",
        "a single endpoint string is a substring test",
    ),
    (
        "substring",
        "true",
        "a string endpoint list is a substring test",
    ),
    ("absent", "false", "a missing endpoint list is unsupported"),
    (
        "empty",
        "false",
        "an empty endpoint list offers no endpoint",
    ),
    ("null", "false", "a null endpoint list is unsupported"),
    ("zero", "false", "a zero endpoint list is not a list"),
    ("false", "false", "a false endpoint list is not a list"),
    ("object", "true", "a mapping is a key test"),
    (
        "object_miss",
        "false",
        "a mapping without the Responses key misses",
    ),
    (
        "number",
        "not-iterable",
        "a number endpoint list is not iterable",
    ),
    (
        "flag",
        "not-iterable",
        "a boolean endpoint list is not iterable",
    ),
    (
        "not_object",
        "false",
        "a non-object entry is skipped, not reported",
    ),
    (
        "chat_only_string",
        "false",
        "a string endpoint list without the Responses endpoint does not match",
    ),
    (
        "chat_only_list",
        "false",
        "a list endpoint list without the Responses endpoint does not match",
    ),
    (
        "raising",
        "not-iterable",
        "the raw key is inspected before the bedrock/-prefixed one",
    ),
    (
        "bedrock/listed",
        "false",
        "a model spelled with the bedrock/ prefix is not stripped before lookup",
    ),
    (
        "prefixed",
        "true",
        "the bedrock/-prefixed key is inspected after the raw one",
    ),
    (
        "only_the_pair",
        "true",
        "the bedrock/-prefixed string entry answers when the raw key is absent",
    ),
    (
        "us.anthropic.claude-alias-v1:0",
        "false",
        "a region-prefixed name is not an alias for its base model",
    ),
    ("missing", "false", "an unmapped model has no endpoints"),
];

const CACHE_POINT_MAP: &str = r#"{
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
        }"#;

const CACHE_POINT_MODELS: &[(&str, &str, &str)] = &[
    (
        "cached",
        "true",
        "the cache-point flag enables explicit points",
    ),
    (
        "uncached",
        "false",
        "a false cache-point flag disables them",
    ),
    ("flag_one", "false", "an integer one is not true"),
    ("empty", "false", "an empty entry carries no flag"),
    ("null_entry", "true", "a null value is not an entry"),
    ("zero_entry", "malformed", "a zero entry is not an object"),
    ("malformed", "malformed", "a number entry is not an object"),
    (
        "openai.gpt-5",
        "false",
        "the OpenAI family never accepts explicit cache points",
    ),
    (
        "us.openai.gpt-5",
        "false",
        "the region prefix keeps the family",
    ),
    (
        "myopenai.gpt-5",
        "true",
        "the family pattern needs a leading dot, slash or start",
    ),
    (
        "bedrock/openai.gpt-5",
        "false",
        "the family pattern also matches after a slash",
    ),
    (
        "us.OpenAI.gpt-5",
        "true",
        "the family pattern is case-sensitive",
    ),
    (
        "us.anthropic.claude-regional-v1:0",
        "true",
        "the base model entry answers when the raw name has none",
    ),
    (
        "us.anthropic.claude-short",
        "true",
        "a true entry short-circuits before a malformed base entry",
    ),
    (
        "us.anthropic.claude-bad",
        "malformed",
        "a malformed raw entry fails before a true base entry",
    ),
    (
        "missing",
        "true",
        "an unmapped model accepts explicit cache points",
    ),
];

const STRICT_TOOLS_PACKAGED: &str = r#"{
            "strict-false": {"bedrock_converse_supports_strict_tools": false},
            "strict-true": {"bedrock_converse_supports_strict_tools": true}
        }"#;

const STRICT_TOOLS_ANSWERS: &[(&str, &str)] = &[
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
        "anthropic.claude-flag-and-key",
        r#"{"bedrock_converse_supports_strict_tools": true, "key": "strict-false"}"#,
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
];

const STRICT_TOOLS_CASES: &[(&str, bool, &str)] = &[
    (
        "anthropic.claude-by-key",
        false,
        "a string key selects the packaged entry and its false flag",
    ),
    (
        "anthropic.claude-by-true-key",
        true,
        "a true key enables it",
    ),
    ("anthropic.claude-flag-true", true, "a true flag enables it"),
    (
        "anthropic.claude-flag-false",
        false,
        "a false flag disables it",
    ),
    (
        "anthropic.claude-flag-and-key",
        true,
        "an actual boolean wins before the packaged key is read",
    ),
    (
        "anthropic.claude-flag-null",
        false,
        "a null flag is not a boolean and falls through to the key",
    ),
    (
        "anthropic.claude-no-key",
        true,
        "a lookup with neither a boolean nor a key leaves the default on",
    ),
    (
        "anthropic.claude-v1",
        true,
        "the base name itself answers before a stripped variant",
    ),
    (
        "anthropic.claude-by-key-v1:0",
        false,
        "the version-stripped candidate answers when the raw name is unmapped",
    ),
    (
        "amazon.nova-pro-v1:0",
        false,
        "a non-Anthropic family never forwards strict tools",
    ),
    (
        "anthropic.claude-unmapped",
        true,
        "every suppressed lookup failure leaves the default on",
    ),
    (
        "anthropic.claude-vnext",
        true,
        "a -v that does not start a numeric version stays in the name",
    ),
    (
        "anthropic.claude-v1:beta",
        true,
        "a non-numeric revision stays in the name",
    ),
    (
        "anthropic.claude-gap-v1:0",
        false,
        "a candidate without a flag or key moves on to the stripped name",
    ),
    (
        "anthropic.claude-nokey-v1:0",
        false,
        "a key with no packaged entry moves on to the stripped name",
    ),
    (
        "anthropic.claude-gap-v\u{661}",
        false,
        "a non-ASCII decimal digit is a version separator too",
    ),
    (
        "anthropic.claude-gap-v1:\u{661}",
        false,
        "a non-ASCII decimal digit is a revision separator too",
    ),
];

const EFFORT_PACKAGED: &str = r#"{
            "ceiling-low": {"bedrock_output_config_effort_ceiling": "low"},
            "ceiling-medium": {"bedrock_output_config_effort_ceiling": "medium"},
            "ceiling-high": {"bedrock_output_config_effort_ceiling": "high"},
            "ceiling-max": {"bedrock_output_config_effort_ceiling": "max"},
            "ceiling-xhigh": {"bedrock_output_config_effort_ceiling": "xhigh"},
            "ceiling-minimal": {"bedrock_output_config_effort_ceiling": "minimal"},
            "ceiling-none": {"bedrock_output_config_effort_ceiling": "none"}
        }"#;

const EFFORT_ANSWERS: &[(&str, &str)] = &[
    ("opus", r#"{"key": "ceiling-high"}"#),
    ("opus-low", r#"{"key": "ceiling-low"}"#),
    ("opus-medium", r#"{"key": "ceiling-medium"}"#),
    (
        "opus-max",
        r#"{"bedrock_output_config_effort_ceiling": "max"}"#,
    ),
    ("opus-xhigh", r#"{"key": "ceiling-xhigh"}"#),
    (
        "info-low",
        r#"{"bedrock_output_config_effort_ceiling": "low"}"#,
    ),
    (
        "info-medium",
        r#"{"bedrock_output_config_effort_ceiling": "medium"}"#,
    ),
    (
        "info-high",
        r#"{"bedrock_output_config_effort_ceiling": "high"}"#,
    ),
    (
        "opus-unknown",
        r#"{"bedrock_output_config_effort_ceiling": "ultra"}"#,
    ),
    (
        "opus-bad-ceiling",
        r#"{"bedrock_output_config_effort_ceiling": "ultra", "key": "ceiling-low"}"#,
    ),
    ("opus-minimal", r#"{"key": "ceiling-minimal"}"#),
    ("opus-none", r#"{"key": "ceiling-none"}"#),
    ("opus-bare", r#"{"key": "missing-entry"}"#),
];

const EFFORT_CASES: &[(&str, &str, &str)] = &[
    (
        "opus",
        r#"{"effort": "xhigh", "format": "x"}"#,
        r#"{"effort": "high", "format": "x"}"#,
    ),
    ("opus", r#"{"effort": "low"}"#, r#"{"effort": "low"}"#),
    ("opus", r#"{"effort": "high"}"#, r#"{"effort": "high"}"#),
    ("opus-low", r#"{"effort": "xhigh"}"#, r#"{"effort": "low"}"#),
    (
        "opus-medium",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "medium"}"#,
    ),
    ("opus-max", r#"{"effort": "xhigh"}"#, r#"{"effort": "max"}"#),
    ("opus-max", r#"{"effort": "max"}"#, r#"{"effort": "max"}"#),
    (
        "opus-xhigh",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    ),
    ("info-low", r#"{"effort": "xhigh"}"#, r#"{"effort": "low"}"#),
    ("info-low", r#"{"effort": "high"}"#, r#"{"effort": "low"}"#),
    (
        "info-low",
        r#"{"effort": "medium"}"#,
        r#"{"effort": "low"}"#,
    ),
    ("info-low", r#"{"effort": "low"}"#, r#"{"effort": "low"}"#),
    (
        "info-medium",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "medium"}"#,
    ),
    (
        "info-medium",
        r#"{"effort": "high"}"#,
        r#"{"effort": "medium"}"#,
    ),
    (
        "info-high",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "high"}"#,
    ),
    ("opus", r#"{"effort": "ultra"}"#, r#"{"effort": "ultra"}"#),
    ("opus", r#"{"effort": null}"#, r#"{"effort": null}"#),
    ("opus", r#"{"effort": 5}"#, r#"{"effort": 5}"#),
    ("opus", r#"{"effort": true}"#, r#"{"effort": true}"#),
    ("opus", "{}", "{}"),
    ("opus", r#""plain""#, r#""plain""#),
    (
        "opus-unknown",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    ),
    (
        "opus-bad-ceiling",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "low"}"#,
    ),
    (
        "opus-minimal",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    ),
    (
        "opus-none",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    ),
    (
        "opus-bare",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    ),
    (
        "unmapped",
        r#"{"effort": "xhigh"}"#,
        r#"{"effort": "xhigh"}"#,
    ),
];

const PARALLEL_TOOLS_MAP: &str = r#"{
            "one": {"supports_parallel_tool_use_config": true},
            "integer": {"supports_parallel_tool_use_config": 1},
            "off": {"supports_parallel_tool_use_config": false},
            "empty": {},
            "null_entry": null,
            "false_entry": false,
            "zero_entry": 0,
            "malformed": 5,
            "text_entry": "",
            "empty_list": [],
            "list_entry": [1],
            "anthropic.claude-p-v1:0": {"supports_parallel_tool_use_config": true},
            "us.anthropic.claude-q-v1:0": {"supports_parallel_tool_use_config": true},
            "anthropic.claude-q-v1:0": 5
        }"#;

const PARALLEL_TOOLS_MODELS: &[(&str, &str, &str)] = &[
    ("one", "true", "a true flag enables parallel tools"),
    ("integer", "false", "an integer one is not a boolean true"),
    ("off", "false", "a false flag disables parallel tools"),
    ("empty", "false", "an empty entry carries no flag"),
    ("null_entry", "false", "a null value is not an entry"),
    ("false_entry", "false", "a false value is not an entry"),
    ("zero_entry", "false", "a zero value is not an entry"),
    ("malformed", "malformed", "a number entry is not an object"),
    ("text_entry", "false", "a text value is not an entry"),
    ("empty_list", "false", "an empty list is not an entry"),
    ("list_entry", "malformed", "a list entry is not an object"),
    (
        "us.anthropic.claude-p-v1:0",
        "true",
        "the base model entry enables it",
    ),
    (
        "us.anthropic.claude-q-v1:0",
        "true",
        "a true entry short-circuits before a malformed base entry",
    ),
    ("missing", "false", "an unmapped model leaves it off"),
];

const EXTENDED_TTL_MAP: &str = r#"{
            "priced": {"cache_creation_input_token_cost_above_1hr": 0.000006},
            "zero": {"cache_creation_input_token_cost_above_1hr": 0},
            "false_flag": {"cache_creation_input_token_cost_above_1hr": false},
            "null_flag": {"cache_creation_input_token_cost_above_1hr": null},
            "absent": {},
            "null_entry": null,
            "malformed": 5,
            "anthropic.claude-e-v1:0": {"cache_creation_input_token_cost_above_1hr": 1}
        }"#;

const EXTENDED_TTL_MODELS: &[(&str, &str, &str)] = &[
    (
        "priced",
        "true",
        "a one-hour price declares the extended TTL",
    ),
    (
        "zero",
        "true",
        "a zero one-hour price still declares the field",
    ),
    (
        "false_flag",
        "true",
        "a false one-hour price still declares the field",
    ),
    ("null_flag", "false", "a null one-hour price is no field"),
    ("absent", "false", "a missing one-hour price is no field"),
    ("null_entry", "false", "a null value is not an entry"),
    ("malformed", "malformed", "a number entry is not an object"),
    (
        "us.anthropic.claude-e-v1:0",
        "true",
        "the base model entry declares it",
    ),
    ("missing", "false", "an unmapped model has no extended TTL"),
];

const fn outcome(result: &Result<bool, CapabilityError>) -> &'static str {
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

fn assert_flags<Gate>(models: &[(&str, &str, &str)], gate: Gate)
where
    Gate: Fn(&str) -> Result<bool, CapabilityError>,
{
    for &(model, expected, message) in models {
        assert_eq!(outcome(&gate(model)), expected, "{message}");
    }
}

pub(super) fn mantle_api_base() {
    for &(api_base, endpoint, environment_base, region, expected) in MANTLE_CASES {
        assert_eq!(
            mantle_messages_url(api_base, endpoint, environment_base, region),
            expected
        );
    }
}

pub(super) fn native_responses() {
    let cost = map(NATIVE_RESPONSES_MAP);
    assert_flags(NATIVE_RESPONSES_MODELS, |model| {
        supports_openai_responses(Some(model), &cost)
    });
    let unnamed = [None, Some("")].map(|name| outcome(&supports_openai_responses(name, &cost)));
    assert_eq!(
        unnamed,
        ["false", "false"],
        "no model name has no endpoints"
    );
}

pub(super) fn cache_point_unknown_model() {
    let cost = map(CACHE_POINT_MAP);
    assert_flags(CACHE_POINT_MODELS, |model| {
        accepts_cache_points(Some(model), &cost)
    });
    assert_eq!(outcome(&accepts_cache_points(None, &cost)), "true");
    assert_eq!(
        outcome(&accepts_cache_points(Some(""), &cost)),
        "true",
        "an empty name is not the missing model"
    );
}

pub(super) fn strict_tools_fallback() {
    let packaged = ModelCatalog::parse(STRICT_TOOLS_PACKAGED.as_bytes(), UnknownFields::Warn)
        .expect("the packaged fixture parses");
    let answers = Answers::new(STRICT_TOOLS_ANSWERS);
    let runtime = runtime();
    let ask =
        |model: &str| runtime.block_on(converse_supports_strict_tools(model, &answers, &packaged));
    for &(model, expected, message) in STRICT_TOOLS_CASES {
        assert_eq!(ask(model), expected, "{message}");
    }
}

pub(super) fn effort_ceiling() {
    let packaged = ModelCatalog::parse(EFFORT_PACKAGED.as_bytes(), UnknownFields::Warn)
        .expect("the packaged fixture parses");
    let answers = Answers::new(EFFORT_ANSWERS);
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
    for &(model, config, expected) in EFFORT_CASES {
        clamped(model, config, expected);
    }
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
    let cost = map(PARALLEL_TOOLS_MAP);
    assert_flags(PARALLEL_TOOLS_MODELS, |model| {
        supports_parallel_tools(model, &cost)
    });
}

pub(super) fn extended_cache_ttl() {
    let cost = map(EXTENDED_TTL_MAP);
    assert_flags(EXTENDED_TTL_MODELS, |model| {
        is_claude_4_5_on_bedrock(model, &cost)
    });
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
