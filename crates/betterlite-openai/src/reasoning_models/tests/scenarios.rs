use serde_json::Value;

use super::super::{
    ReasoningError, ReasoningPolicy, audio, effort_resolves_to_none, gpt_series_version,
    is_model_gpt_5_2_model, is_model_gpt_5_4_plus_model, is_model_gpt_5_6_plus_model,
    is_model_gpt_6_plus_model, o_series, select_policy,
};
use super::{
    DropParams, Fixture, global_drop, gpt5_supported, map_gpt5, no_drop, request, request_drop,
};
use crate::chat::{ChatMessage, MessageContent, Role};
use crate::openai_base_config::{CatalogCapabilities, Object};

const SEARCH_SUPPORTED: [&str; 13] = [
    "max_tokens",
    "max_completion_tokens",
    "stream",
    "stream_options",
    "web_search_options",
    "service_tier",
    "safety_identifier",
    "response_format",
    "user",
    "store",
    "verbosity",
    "max_retries",
    "extra_headers",
];

const GPT_ENTRIES: &str = r#"{
    "gpt-5.1": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "supports_xhigh_reasoning_effort": false,
        "default_reasoning_effort": "none"
    },
    "gpt-5.2": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "supports_xhigh_reasoning_effort": true,
        "default_reasoning_effort": "none"
    },
    "gpt-5": {
        "litellm_provider": "openai",
        "supports_tool_choice": false,
        "supports_none_reasoning_effort": false
    },
    "gpt-5.5": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "default_reasoning_effort": "high"
    },
    "gpt-5.5-pro": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "supports_low_reasoning_effort": false,
        "supports_minimal_reasoning_effort": false,
        "default_reasoning_effort": "none"
    }
}"#;

const GPT_MODELS: [&str; 5] = ["gpt-5.1", "gpt-5.2", "gpt-5", "gpt-5.5", "gpt-5.5-pro"];

fn request_value(source: &str) -> Value {
    serde_json::from_str(source).expect("fixture json")
}

pub(super) async fn gpt5_search_allowlist() {
    let mut fixture = Fixture::new(&["gpt-5-search-api"], "{}");
    assert_eq!(
        gpt5_supported(&mut fixture, "gpt-5-search-api").await,
        SEARCH_SUPPORTED
    );
    assert_eq!(
        gpt5_supported(&mut fixture, "proxy/gpt-5-search").await,
        SEARCH_SUPPORTED
    );
    search_mapping(&mut fixture).await;
}

async fn search_mapping(fixture: &mut Fixture) {
    let mut non_default = request(
        r#"{"max_tokens": 4, "temperature": 0.2, "reasoning_effort": "high",
            "web_search_options": {"search_context_size": "low"}, "store": true,
            "service_tier": "default"}"#,
    );
    let mut optional = request(r#"{"max_tokens": 9, "user": "caller"}"#);
    map_gpt5(
        fixture,
        "gpt-5-search-api",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("search models skip the reasoning guards");
    assert_eq!(
        optional.get("max_completion_tokens"),
        Some(&Value::from(4_i64))
    );
    assert_eq!(optional.get("max_tokens"), Some(&Value::from(9_i64)));
    assert_eq!(optional.get("user"), Some(&Value::from("caller")));
    assert_eq!(
        optional.get("web_search_options"),
        Some(&request_value(r#"{"search_context_size": "low"}"#))
    );
    assert_eq!(optional.get("store"), Some(&Value::from(true)));
    assert_eq!(optional.get("service_tier"), Some(&Value::from("default")));
    assert!(!optional.contains_key("temperature"));
    assert!(!optional.contains_key("reasoning_effort"));
    assert_eq!(non_default.get("temperature"), Some(&Value::from(0.2_f64)));
    assert_eq!(
        non_default.get("reasoning_effort"),
        Some(&Value::from("high"))
    );
}

pub(super) fn gpt5_chat_exclusion() {
    let catalog = Fixture::new(
        &[
            "gpt-5-chat",
            "gpt-5-search-api",
            "o3",
            "gpt-audio",
            "gpt-4o",
        ],
        "{}",
    )
    .catalog;
    assert_eq!(select_policy("gpt-5-chat", &catalog), ReasoningPolicy::Base);
    assert_eq!(
        select_policy("proxy/gpt-5-chat", &catalog),
        ReasoningPolicy::Base
    );
    assert_eq!(
        select_policy("gpt-5-search-api", &catalog),
        ReasoningPolicy::Gpt5Search
    );
    assert_eq!(select_policy("gpt-5.1", &catalog), ReasoningPolicy::Gpt5);
    o_series_and_audio_policies(&catalog);
    assert!(!super::super::is_model_gpt_5_search_model("gpt-5-chat"));
    assert!(super::super::is_model_gpt_5_search_model(
        "gpt-5-chat-search"
    ));
    assert_eq!(
        select_policy("gpt-5-chat-search", &catalog),
        ReasoningPolicy::Base
    );
}

fn o_series_and_audio_policies(catalog: &CatalogCapabilities) {
    assert_eq!(select_policy("o3", catalog), ReasoningPolicy::OSeries);
    assert_eq!(
        select_policy("openai/o3", catalog),
        ReasoningPolicy::OSeries
    );
    assert_eq!(select_policy("o4-mini", catalog), ReasoningPolicy::Base);
    assert_eq!(select_policy("gpt-audio", catalog), ReasoningPolicy::Audio);
    assert_eq!(
        select_policy("gpt-audio-mini", catalog),
        ReasoningPolicy::Base
    );
    assert_eq!(select_policy("gpt-4o", catalog), ReasoningPolicy::Base);
}

pub(super) async fn gpt6_family_selection() {
    let mut fixture = Fixture::new(&["azure_ai/gpt-6-astra", "gpt-6"], "{}");
    let catalog = fixture.catalog.clone();
    assert_eq!(
        select_policy("azure_ai/gpt-6-astra", &catalog),
        ReasoningPolicy::Gpt5
    );
    assert_eq!(select_policy("gpt-6", &catalog), ReasoningPolicy::Gpt5);
    assert_eq!(gpt_series_version("azure_ai/gpt-6-astra"), Some((6, 0)));
    assert!(is_model_gpt_5_4_plus_model("gpt-6"));
    assert!(is_model_gpt_5_6_plus_model("gpt-6"));
    assert!(is_model_gpt_6_plus_model("gpt-6"));
    assert!(!is_model_gpt_6_plus_model("gpt-5.6"));
    assert!(!is_model_gpt_5_2_model("gpt-6"));
    let supported = gpt5_supported(&mut fixture, "gpt-6").await;
    assert!(supported.contains(&"reasoning_effort") && supported.contains(&"verbosity"));
}

pub(super) async fn dict_effort_normalization() {
    let mut fixture = Fixture::new(&GPT_MODELS, GPT_ENTRIES);
    dict_effort_dictionaries(&mut fixture).await;
    dict_effort_fallbacks(&mut fixture).await;
}

async fn dict_effort_dictionaries(fixture: &mut Fixture) {
    let mut non_default =
        request(r#"{"reasoning_effort": {"effort": "high", "summary": "detailed"}}"#);
    let mut optional = request("{}");
    map_gpt5(
        fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a dictionary effort normalizes");
    assert_eq!(
        non_default.get("reasoning_effort"),
        Some(&Value::from("high"))
    );
    assert_eq!(optional.get("reasoning_effort"), Some(&Value::from("high")));

    let mut empty_probe = request("{}");
    let mut optional_only = request(r#"{"reasoning_effort": {"effort": "low"}}"#);
    map_gpt5(
        fixture,
        "gpt-5.1",
        &mut empty_probe,
        &mut optional_only,
        no_drop(),
    )
    .await
    .expect("an optional dictionary effort normalizes");
    assert!(!empty_probe.contains_key("reasoning_effort"));
    assert_eq!(
        optional_only.get("reasoning_effort"),
        Some(&Value::from("low"))
    );
}

async fn dict_effort_fallbacks(fixture: &mut Fixture) {
    let mut empty_dictionary = request(r#"{"reasoning_effort": {}}"#);
    let mut declared = request(r#"{"reasoning_effort": "high"}"#);
    map_gpt5(
        fixture,
        "gpt-5.1",
        &mut empty_dictionary,
        &mut declared,
        no_drop(),
    )
    .await
    .expect("a dictionary without effort resolves none");
    assert_eq!(
        empty_dictionary.get("reasoning_effort"),
        Some(&request_value("{}"))
    );
    assert_eq!(declared.get("reasoning_effort"), Some(&request_value("{}")));

    let mut absent = request(r#"{"reasoning_effort": null}"#);
    let mut given = request(r#"{"reasoning_effort": "low"}"#);
    map_gpt5(fixture, "gpt-5.1", &mut absent, &mut given, no_drop())
        .await
        .expect("a null effort falls back to the optional one");
    assert_eq!(absent.get("reasoning_effort"), Some(&Value::Null));
    assert_eq!(given.get("reasoning_effort"), Some(&Value::Null));

    let mut null_effort = request(r#"{"reasoning_effort": null}"#);
    let mut guarded = request(r#"{"reasoning_effort": "xhigh"}"#);
    let refused = map_gpt5(
        fixture,
        "gpt-5.1",
        &mut null_effort,
        &mut guarded,
        no_drop(),
    )
    .await
    .expect_err("the optional effort guards when the request effort is null");
    assert_eq!(refused.status_code(), 400);
}

async fn effort_maps(
    fixture: &mut Fixture,
    model: &str,
    effort: &str,
    drop_params: DropParams,
    existing: bool,
) -> Result<(Object, Object), ReasoningError> {
    let mut non_default = request(&format!(r#"{{"reasoning_effort": "{effort}"}}"#));
    let mut optional = if existing {
        request(&format!(r#"{{"reasoning_effort": "{effort}"}}"#))
    } else {
        request("{}")
    };
    map_gpt5(fixture, model, &mut non_default, &mut optional, drop_params).await?;
    Ok((non_default, optional))
}

async fn assert_rejected(fixture: &mut Fixture, model: &str, effort: &str) {
    let failure = effort_maps(fixture, model, effort, no_drop(), false)
        .await
        .expect_err("an explicit false rejects the effort");
    assert_eq!(failure.status_code(), 400);
}

async fn assert_dropped(fixture: &mut Fixture, model: &str, effort: &str) {
    for drop_params in [request_drop(), global_drop()] {
        let (non_default, optional) = effort_maps(fixture, model, effort, drop_params, true)
            .await
            .expect("drop_params removes the effort");
        assert!(!non_default.contains_key("reasoning_effort"));
        assert!(!optional.contains_key("reasoning_effort"));
    }
}

pub(super) async fn xhigh_opt_in() {
    let mut fixture = Fixture::new(&GPT_MODELS, GPT_ENTRIES);
    let (_, optional) = effort_maps(&mut fixture, "gpt-5.2", "xhigh", no_drop(), false)
        .await
        .expect("xhigh is declared");
    assert_eq!(
        optional.get("reasoning_effort"),
        Some(&Value::from("xhigh"))
    );
    let failure = effort_maps(&mut fixture, "gpt-5.1", "xhigh", no_drop(), false)
        .await
        .expect_err("xhigh needs an explicit capability");
    assert_eq!(
        failure,
        ReasoningError::UnsupportedEffort {
            effort: String::from("xhigh")
        }
    );
    assert_dropped(&mut fixture, "gpt-5.1", "xhigh").await;
    xhigh_dictionary(&mut fixture).await;
}

async fn xhigh_dictionary(fixture: &mut Fixture) {
    let mut non_default = request(r#"{"reasoning_effort": {"effort": "xhigh"}}"#);
    let mut optional = request("{}");
    map_gpt5(
        fixture,
        "gpt-5.2",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a dictionary effort normalizes before the guard");
    assert_eq!(
        non_default.get("reasoning_effort"),
        Some(&Value::from("xhigh"))
    );
    assert_eq!(
        optional.get("reasoning_effort"),
        Some(&Value::from("xhigh"))
    );
}

pub(super) async fn low_minimal_opt_out() {
    let mut fixture = Fixture::new(&GPT_MODELS, GPT_ENTRIES);
    let (_, optional) = effort_maps(&mut fixture, "gpt-5.1", "low", no_drop(), false)
        .await
        .expect("an unknown low capability passes through");
    assert_eq!(optional.get("reasoning_effort"), Some(&Value::from("low")));
    assert_rejected(&mut fixture, "gpt-5.5-pro", "low").await;
    assert_rejected(&mut fixture, "gpt-5.5-pro", "minimal").await;
    assert_dropped(&mut fixture, "gpt-5.5-pro", "minimal").await;
    let (removed_non_default, removed_optional) =
        effort_maps(&mut fixture, "gpt-5.5-pro", "low", global_drop(), true)
            .await
            .expect("the global drop removes a disabled low effort");
    assert!(!removed_non_default.contains_key("reasoning_effort"));
    assert!(!removed_optional.contains_key("reasoning_effort"));
}

pub(super) async fn catalog_before_default_key() {
    assert!(
        !resolves(
            r#"{
            "gpt-5.1": {"litellm_provider": "openai", "supports_none_reasoning_effort": true},
            "other-model": {"litellm_provider": "openai", "default_reasoning_effort": "high"}
        }"#
        )
        .await
    );
    assert!(
        resolves(
            r#"{"gpt-5.1": {"litellm_provider": "openai", "default_reasoning_effort": "none"}}"#
        )
        .await
    );
    assert!(
        !resolves(
            r#"{"gpt-5.1": {"litellm_provider": "openai", "default_reasoning_effort": "high"}}"#
        )
        .await
    );
    assert!(
        resolves(
            r#"{"gpt-5.1": {"litellm_provider": "openai", "supports_none_reasoning_effort": true}}"#
        )
        .await
    );
    assert!(!resolves(
        r#"{"gpt-5.1": {"litellm_provider": "openai", "supports_none_reasoning_effort": false}}"#
    )
    .await);
}

async fn resolves(entries: &str) -> bool {
    let mut fixture = Fixture::new(&["gpt-5.1"], entries);
    effort_resolves_to_none(&mut fixture, "gpt-5.1", None).await
}

pub(super) async fn temperature_while_thinking() {
    let mut fixture = Fixture::new(&GPT_MODELS, GPT_ENTRIES);
    temperature_omissions(&mut fixture).await;
    temperature_none_default(&mut fixture).await;
    temperature_forbidden(&mut fixture).await;
}

async fn temperature_omissions(fixture: &mut Fixture) {
    let mut non_default = request(r#"{"temperature": null}"#);
    let mut optional = request("{}");
    map_gpt5(
        fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a null temperature is omitted");
    assert!(!non_default.contains_key("temperature"));
    assert!(!optional.contains_key("temperature"));

    let mut kept = request(r#"{"temperature": 1, "reasoning_effort": "high"}"#);
    let mut stored = request("{}");
    map_gpt5(fixture, "gpt-5.1", &mut kept, &mut stored, no_drop())
        .await
        .expect("temperature 1 always survives");
    assert_eq!(stored.get("temperature"), Some(&Value::from(1_i64)));
}

async fn temperature_none_default(fixture: &mut Fixture) {
    let mut non_default = request(r#"{"temperature": 0.5}"#);
    let mut optional = request("{}");
    map_gpt5(
        fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a declared default of none permits a temperature");
    assert_eq!(optional.get("temperature"), Some(&Value::from(0.5_f64)));

    let mut existing = request("{}");
    let mut carried = request(r#"{"temperature": 0.3}"#);
    map_gpt5(fixture, "gpt-5.1", &mut existing, &mut carried, no_drop())
        .await
        .expect("an existing optional temperature is untouched");
    assert_eq!(carried.get("temperature"), Some(&Value::from(0.3_f64)));
}

async fn temperature_forbidden(fixture: &mut Fixture) {
    let mut non_default = request(r#"{"temperature": 0.5}"#);
    let mut optional = request("{}");
    let failure = map_gpt5(
        fixture,
        "gpt-5.5",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect_err("a declared default of high forbids a temperature");
    assert_eq!(failure.status_code(), 400);

    let mut dropped = request(r#"{"temperature": 0.5}"#);
    let mut untouched = request("{}");
    map_gpt5(
        fixture,
        "gpt-5.5",
        &mut dropped,
        &mut untouched,
        request_drop(),
    )
    .await
    .expect("drop_params removes a forbidden temperature");
    assert!(!dropped.contains_key("temperature"));
    assert!(!untouched.contains_key("temperature"));

    let mut unresolved = request(r#"{"temperature": 0.5, "reasoning_effort": "none"}"#);
    let mut empty = request("{}");
    let refused = map_gpt5(fixture, "gpt-5", &mut unresolved, &mut empty, no_drop())
        .await
        .expect_err("a model that cannot resolve to none rejects a temperature");
    assert_eq!(refused.status_code(), 400);
}

pub(super) async fn sampling_with_none() {
    let mut fixture = Fixture::new(&GPT_MODELS, GPT_ENTRIES);
    sampling_rejection(&mut fixture).await;
    sampling_permits(&mut fixture).await;
    sampling_without_none(&mut fixture).await;
}

async fn sampling_rejection(fixture: &mut Fixture) {
    let mut non_default =
        request(r#"{"logprobs": true, "top_p": 0.9, "reasoning_effort": "high"}"#);
    let mut optional = request("{}");
    let failure = map_gpt5(
        fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect_err("sampling needs effort none");
    assert_eq!(failure.status_code(), 400);

    let mut dropped = request(r#"{"logprobs": true, "top_p": 0.9, "reasoning_effort": "high"}"#);
    let mut untouched = request("{}");
    map_gpt5(
        fixture,
        "gpt-5.1",
        &mut dropped,
        &mut untouched,
        request_drop(),
    )
    .await
    .expect("drop_params removes the sampling params");
    assert!(!dropped.contains_key("logprobs"));
    assert!(!dropped.contains_key("top_p"));
    assert!(!untouched.contains_key("logprobs"));
}

async fn sampling_permits(fixture: &mut Fixture) {
    let mut explicit = request(r#"{"logprobs": true, "reasoning_effort": "none"}"#);
    let mut optional = request("{}");
    map_gpt5(fixture, "gpt-5.1", &mut explicit, &mut optional, no_drop())
        .await
        .expect("an explicit none effort permits sampling");
    assert_eq!(optional.get("logprobs"), Some(&Value::from(true)));

    let mut declared = request(r#"{"top_p": 0.5}"#);
    let mut stored = request("{}");
    map_gpt5(fixture, "gpt-5.1", &mut declared, &mut stored, no_drop())
        .await
        .expect("a declared default of none permits sampling");
    assert_eq!(stored.get("top_p"), Some(&Value::from(0.5_f64)));

    let mut empty = request("{}");
    let mut carried = request(r#"{"logprobs": true, "top_p": 0.4}"#);
    map_gpt5(fixture, "gpt-5.1", &mut empty, &mut carried, no_drop())
        .await
        .expect("existing optional sampling values are not popped");
    assert_eq!(carried.get("logprobs"), Some(&Value::from(true)));
    assert_eq!(carried.get("top_p"), Some(&Value::from(0.4_f64)));
}

async fn sampling_without_none(fixture: &mut Fixture) {
    let mut non_default = request(r#"{"logprobs": true, "top_p": 0.5}"#);
    let mut optional = request("{}");
    map_gpt5(fixture, "gpt-5", &mut non_default, &mut optional, no_drop())
        .await
        .expect("a model without none support never enters the sampling guard");
    assert!(!optional.contains_key("logprobs"));
    assert!(!optional.contains_key("top_p"));
    assert_eq!(non_default.get("logprobs"), Some(&Value::from(true)));

    let mut empty = request("{}");
    let mut carried = request(r#"{"reasoning_effort": "high", "logprobs": true}"#);
    map_gpt5(fixture, "gpt-5.1", &mut empty, &mut carried, no_drop())
        .await
        .expect("the sampling branch reads only non_default_params");
    assert_eq!(carried.get("logprobs"), Some(&Value::from(true)));
}

pub(super) async fn o1_developer_roles() {
    let messages = request_messages();
    let mut rejecting = Fixture::new(
        &["o3"],
        r#"{"o3": {"litellm_provider": "openai", "supports_system_messages": false}}"#,
    );
    let first = o_series::transform_messages(&mut rejecting, "o3", &messages).await;
    let second = o_series::transform_messages(&mut rejecting, "o3", &messages).await;
    assert_eq!(first, second, "one transform serves both paths");
    assert_eq!(first[0].role, Role::User);
    assert_eq!(
        first[0].content,
        Some(MessageContent::Text(String::from("be brief")))
    );
    assert_eq!(first[0].name, None);
    assert_eq!(first[1].role, Role::Developer);
    assert_eq!(first[2], messages[2]);

    let mut accepting = Fixture::new(
        &["o3"],
        r#"{"o3": {"litellm_provider": "openai", "supports_system_messages": true}}"#,
    );
    let kept = o_series::transform_messages(&mut accepting, "o3", &messages).await;
    assert_eq!(kept, messages);
}

fn request_messages() -> Vec<ChatMessage> {
    serde_json::from_str(
        r#"[{"role": "system", "content": "be brief", "name": "instruction"},
            {"role": "developer", "content": "use the tools"},
            {"role": "user", "content": "hello"}]"#,
    )
    .expect("messages")
}

pub(super) fn audio_token_parameter() {
    let fixture = Fixture::new(
        &["gpt-audio"],
        r#"{"gpt-audio": {"litellm_provider": "openai", "supports_tool_choice": true}}"#,
    );
    let supported = audio::supported_params("gpt-audio", &fixture.catalog);
    assert_eq!(
        supported.iter().filter(|param| **param == "audio").count(),
        2
    );
    assert!(supported.contains(&"max_tokens") && supported.contains(&"user"));
    assert!(!supported.contains(&"reasoning_effort"));
    let mut non_default = request(r#"{"max_tokens": 7, "audio": {"voice": "alloy"}}"#);
    let mut optional = request(r#"{"existing": true}"#);
    audio::map_params(
        "gpt-audio",
        &mut non_default,
        &mut optional,
        &fixture.catalog,
    );
    assert_eq!(optional.get("max_tokens"), Some(&Value::from(7_i64)));
    assert!(!optional.contains_key("max_completion_tokens"));
    assert_eq!(optional.get("existing"), Some(&Value::from(true)));
}

pub(super) async fn gpt5_regular_allowlist() {
    let mut fixture = Fixture::new(&GPT_MODELS, GPT_ENTRIES);
    gpt5_allowlist_present(&mut fixture).await;
    gpt5_allowlist_limited(&mut fixture).await;
    gpt5_allowlist_unknown(&mut fixture).await;
}

async fn gpt5_allowlist_present(fixture: &mut Fixture) {
    let allowed = gpt5_supported(fixture, "gpt-5.1").await;
    assert_eq!(
        allowed
            .iter()
            .filter(|param| **param == "reasoning_effort")
            .count(),
        1
    );
    for present in [
        "verbosity",
        "tool_choice",
        "logprobs",
        "top_p",
        "top_logprobs",
        "response_format",
        "user",
        "stream",
        "stream_options",
        "max_completion_tokens",
    ] {
        assert!(allowed.contains(&present), "{present} should be allowed");
    }
    for absent in [
        "presence_penalty",
        "frequency_penalty",
        "stop",
        "logit_bias",
        "modalities",
        "prediction",
        "audio",
        "web_search_options",
    ] {
        assert!(!allowed.contains(&absent), "{absent} should be omitted");
    }
}

async fn gpt5_allowlist_limited(fixture: &mut Fixture) {
    let limited = gpt5_supported(fixture, "gpt-5").await;
    assert!(!limited.contains(&"tool_choice"));
    assert!(!limited.contains(&"logprobs"));
    assert!(!limited.contains(&"top_p"));
    assert!(!limited.contains(&"top_logprobs"));
    assert!(limited.contains(&"reasoning_effort") && limited.contains(&"user"));
}

async fn gpt5_allowlist_unknown(fixture: &mut Fixture) {
    let unknown = gpt5_supported(fixture, "custom-gpt-5").await;
    assert!(!unknown.contains(&"user"));
    assert!(!unknown.contains(&"tool_choice"));
}

pub(super) fn version_boundaries() {
    version_parses_series();
    version_rejects_suffixes();
    version_5_4_plus_models();
    version_5_6_plus_models();
    version_6_plus_models();
    version_5_2_models();
}

fn version_parses_series() {
    assert_eq!(gpt_series_version("gpt-5"), Some((5, 0)));
    assert_eq!(gpt_series_version("gpt-5.3"), Some((5, 3)));
    assert_eq!(gpt_series_version("gpt-5.4"), Some((5, 4)));
    assert_eq!(gpt_series_version("gpt-5.6"), Some((5, 6)));
    assert_eq!(gpt_series_version("gpt-6"), Some((6, 0)));
    assert_eq!(gpt_series_version("gpt-6.1"), Some((6, 1)));
    assert_eq!(gpt_series_version("proxy/gpt-5.4"), Some((5, 4)));
    assert_eq!(gpt_series_version("gpt-5.4.1"), Some((5, 4)));
    assert_eq!(gpt_series_version("gpt-5."), Some((5, 0)));
    assert_eq!(gpt_series_version("gpt-5-2025"), Some((5, 0)));
    assert_eq!(gpt_series_version("gpt-05.04"), Some((5, 4)));
    assert_eq!(gpt_series_version("gpt-5.4x"), Some((5, 0)));
}

fn version_rejects_suffixes() {
    assert_eq!(gpt_series_version("gpt-5x"), None);
    assert_eq!(gpt_series_version("GPT-5.4"), None);
    assert_eq!(gpt_series_version("gpt-"), None);
    assert_eq!(gpt_series_version("gpt-5/"), None);
}

fn version_5_4_plus_models() {
    assert!(!is_model_gpt_5_4_plus_model("gpt-5.3"));
    assert!(is_model_gpt_5_4_plus_model("gpt-5.4"));
    assert!(is_model_gpt_5_4_plus_model("gpt-5.39"));
    assert!(!is_model_gpt_5_4_plus_model("gpt-4.9"));
}

fn version_5_6_plus_models() {
    assert!(!is_model_gpt_5_6_plus_model("gpt-5.5"));
    assert!(is_model_gpt_5_6_plus_model("gpt-5.6"));
    assert!(is_model_gpt_5_6_plus_model("gpt-6.0"));
}

fn version_6_plus_models() {
    assert!(!is_model_gpt_6_plus_model("gpt-5.6"));
    assert!(is_model_gpt_6_plus_model("gpt-6"));
}

fn version_5_2_models() {
    assert!(is_model_gpt_5_2_model("gpt-5.2"));
    assert!(is_model_gpt_5_2_model("gpt-5.2-pro"));
    assert!(is_model_gpt_5_2_model("gpt-5.40"));
    assert!(!is_model_gpt_5_2_model("gpt-5.3"));
    assert!(!is_model_gpt_5_2_model("gpt-6"));
}
