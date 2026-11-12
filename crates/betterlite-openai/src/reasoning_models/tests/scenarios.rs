use serde_json::{Value, json};

use super::super::{
    ReasoningError, ReasoningPolicy, audio, effort_resolves_to_none, gpt_series_version,
    is_model_gpt_5_2_model, is_model_gpt_5_4_plus_model, is_model_gpt_5_6_plus_model,
    is_model_gpt_6_plus_model, o_series, select_policy,
};
use super::{
    DropParams, Fixture, Object, global_drop, gpt5_supported, map_gpt5, no_drop, object,
    request_drop,
};

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

const GPT_5_1: &str = r#"{
    "gpt-5.1": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "supports_xhigh_reasoning_effort": false,
        "default_reasoning_effort": "none"
    }
}"#;

pub(super) async fn gpt5_search_allowlist() {
    let mut fixture = Fixture::new(&["gpt-5-search-api"], "{}");
    let supported = gpt5_supported(&mut fixture, "gpt-5-search-api").await;
    assert_eq!(supported, SEARCH_SUPPORTED);
    assert_eq!(
        gpt5_supported(&mut fixture, "proxy/gpt-5-search").await,
        SEARCH_SUPPORTED
    );
    let mut non_default = object(json!({
        "max_tokens": 4,
        "temperature": 0.2,
        "reasoning_effort": "high",
        "web_search_options": {"search_context_size": "low"},
        "store": true,
        "service_tier": "default"
    }));
    let mut optional = object(json!({"max_tokens": 9, "user": "caller"}));
    map_gpt5(
        &mut fixture,
        "gpt-5-search-api",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("search models skip the reasoning guards");
    assert_eq!(optional.get("max_completion_tokens"), Some(&json!(4)));
    assert_eq!(optional.get("max_tokens"), Some(&json!(9)));
    assert_eq!(optional.get("user"), Some(&json!("caller")));
    assert_eq!(
        optional.get("web_search_options"),
        Some(&json!({"search_context_size": "low"}))
    );
    assert_eq!(optional.get("store"), Some(&json!(true)));
    assert_eq!(optional.get("service_tier"), Some(&json!("default")));
    assert!(!optional.contains_key("temperature"));
    assert!(!optional.contains_key("reasoning_effort"));
    assert_eq!(non_default.get("temperature"), Some(&json!(0.2)));
    assert_eq!(non_default.get("reasoning_effort"), Some(&json!("high")));
}

pub(super) fn gpt5_chat_exclusion() {
    let catalog = Fixture::new(&["gpt-5-chat", "gpt-5-search-api"], "{}").catalog;
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
    assert!(!super::super::is_model_gpt_5_search_model("gpt-5-chat"));
    assert!(super::super::is_model_gpt_5_search_model(
        "gpt-5-chat-search"
    ));
    assert_eq!(
        select_policy("gpt-5-chat-search", &catalog),
        ReasoningPolicy::Base
    );
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
    let mut fixture = Fixture::new(&["gpt-5.1"], GPT_5_1);
    let mut non_default =
        object(json!({"reasoning_effort": {"effort": "high", "summary": "detailed"}}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a dictionary effort normalizes");
    assert_eq!(non_default.get("reasoning_effort"), Some(&json!("high")));
    assert_eq!(optional.get("reasoning_effort"), Some(&json!("high")));

    let mut non_default = object(json!({}));
    let mut optional = object(json!({"reasoning_effort": {"effort": "low"}}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("an optional dictionary effort normalizes");
    assert!(!non_default.contains_key("reasoning_effort"));
    assert_eq!(optional.get("reasoning_effort"), Some(&json!("low")));

    let mut non_default = object(json!({"reasoning_effort": ""}));
    let mut optional = object(json!({"reasoning_effort": {"effort": "none"}}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("an empty effort is used as given");
    assert_eq!(non_default.get("reasoning_effort"), Some(&json!("")));
    assert_eq!(optional.get("reasoning_effort"), Some(&json!("")));

    let mut non_default = object(json!({"reasoning_effort": {}}));
    let mut optional = object(json!({"reasoning_effort": "high"}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a dictionary without effort resolves none");
    assert_eq!(non_default.get("reasoning_effort"), Some(&json!({})));
    assert_eq!(optional.get("reasoning_effort"), Some(&json!({})));

    let mut non_default = object(json!({"reasoning_effort": null}));
    let mut optional = object(json!({"reasoning_effort": "low"}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a null effort falls back to the optional one");
    assert_eq!(non_default.get("reasoning_effort"), Some(&Value::Null));
    assert_eq!(optional.get("reasoning_effort"), Some(&Value::Null));
}

const XHIGH_ENTRIES: &str = r#"{
    "gpt-5.2": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "supports_xhigh_reasoning_effort": true,
        "default_reasoning_effort": "none"
    },
    "gpt-5.1": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "supports_xhigh_reasoning_effort": false,
        "default_reasoning_effort": "none"
    }
}"#;

const LOW_ENTRIES: &str = r#"{
    "gpt-5.1": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "default_reasoning_effort": "none"
    },
    "gpt-5.5-pro": {
        "litellm_provider": "openai",
        "supports_tool_choice": true,
        "supports_none_reasoning_effort": true,
        "supports_low_reasoning_effort": false,
        "supports_minimal_reasoning_effort": false
    }
}"#;

async fn effort_maps(
    fixture: &mut Fixture,
    model: &str,
    effort: &str,
    drop_params: DropParams,
    existing: bool,
) -> Result<(Object, Object), ReasoningError> {
    let mut non_default = object(json!({"reasoning_effort": effort}));
    let mut optional = object(json!({}));
    if existing {
        optional.insert(String::from("reasoning_effort"), json!(effort));
    }
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
    let mut fixture = Fixture::new(&["gpt-5.2", "gpt-5.1"], XHIGH_ENTRIES);
    let (_, optional) = effort_maps(&mut fixture, "gpt-5.2", "xhigh", no_drop(), false)
        .await
        .expect("xhigh is declared");
    assert_eq!(optional.get("reasoning_effort"), Some(&json!("xhigh")));
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
    let mut non_default = object(json!({"reasoning_effort": {"effort": "xhigh"}}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.2",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a dictionary effort normalizes before the guard");
    assert_eq!(non_default.get("reasoning_effort"), Some(&json!("xhigh")));
    assert_eq!(optional.get("reasoning_effort"), Some(&json!("xhigh")));
}

pub(super) async fn low_minimal_opt_out() {
    let mut fixture = Fixture::new(&["gpt-5.1", "gpt-5.5-pro"], LOW_ENTRIES);
    let (_, optional) = effort_maps(&mut fixture, "gpt-5.1", "low", no_drop(), false)
        .await
        .expect("an unknown low capability passes through");
    assert_eq!(optional.get("reasoning_effort"), Some(&json!("low")));
    assert_rejected(&mut fixture, "gpt-5.5-pro", "low").await;
    assert_rejected(&mut fixture, "gpt-5.5-pro", "minimal").await;
    assert_dropped(&mut fixture, "gpt-5.5-pro", "minimal").await;
    let (non_default, optional) =
        effort_maps(&mut fixture, "gpt-5.5-pro", "low", global_drop(), true)
            .await
            .expect("the global drop removes a disabled low effort");
    assert!(!non_default.contains_key("reasoning_effort"));
    assert!(!optional.contains_key("reasoning_effort"));
}

pub(super) async fn catalog_before_default_key() {
    let mut fixture = Fixture::new(
        &["gpt-5.1"],
        r#"{
            "gpt-5.1": {"litellm_provider": "openai", "supports_none_reasoning_effort": true},
            "other-model": {"litellm_provider": "openai", "default_reasoning_effort": "high"}
        }"#,
    );
    assert!(
        !effort_resolves_to_none(&mut fixture, "gpt-5.1", None).await,
        "another entry's declared default makes the model's silence mean no"
    );

    let mut fixture = Fixture::new(
        &["gpt-5.1"],
        r#"{"gpt-5.1": {"litellm_provider": "openai", "default_reasoning_effort": "none", "supports_none_reasoning_effort": false}}"#,
    );
    assert!(effort_resolves_to_none(&mut fixture, "gpt-5.1", None).await);

    let mut fixture = Fixture::new(
        &["gpt-5.1"],
        r#"{"gpt-5.1": {"litellm_provider": "openai", "default_reasoning_effort": "high", "supports_none_reasoning_effort": true}}"#,
    );
    assert!(!effort_resolves_to_none(&mut fixture, "gpt-5.1", None).await);

    let mut fixture = Fixture::new(
        &["gpt-5.1"],
        r#"{"gpt-5.1": {"litellm_provider": "openai", "supports_none_reasoning_effort": true}}"#,
    );
    assert!(
        effort_resolves_to_none(&mut fixture, "gpt-5.1", None).await,
        "a catalogue that never heard of the key falls back to the capability"
    );

    let mut fixture = Fixture::new(
        &["gpt-5.1"],
        r#"{"gpt-5.1": {"litellm_provider": "openai", "supports_none_reasoning_effort": false}}"#,
    );
    assert!(!effort_resolves_to_none(&mut fixture, "gpt-5.1", None).await);

    let mut fixture = Fixture::new(&["gpt-5.1"], "{}");
    assert!(effort_resolves_to_none(&mut fixture, "gpt-5.1", Some("none")).await);
    assert!(!effort_resolves_to_none(&mut fixture, "gpt-5.1", Some("low")).await);
}

pub(super) async fn temperature_while_thinking() {
    let entries = r#"{
        "gpt-5.1": {
            "litellm_provider": "openai",
            "supports_tool_choice": true,
            "supports_none_reasoning_effort": true,
            "default_reasoning_effort": "none"
        },
        "gpt-5.5": {
            "litellm_provider": "openai",
            "supports_tool_choice": true,
            "supports_none_reasoning_effort": true,
            "default_reasoning_effort": "high"
        },
        "gpt-5": {
            "litellm_provider": "openai",
            "supports_tool_choice": true,
            "supports_none_reasoning_effort": false
        }
    }"#;
    let mut fixture = Fixture::new(&["gpt-5.1", "gpt-5.5", "gpt-5"], entries);
    let mut non_default = object(json!({"temperature": null}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a null temperature is omitted");
    assert!(!non_default.contains_key("temperature"));
    assert!(!optional.contains_key("temperature"));

    let mut non_default = object(json!({"temperature": 1, "reasoning_effort": "high"}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("temperature 1 always survives");
    assert_eq!(optional.get("temperature"), Some(&json!(1)));

    let mut non_default = object(json!({"temperature": 0.5}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a declared default of none permits a temperature");
    assert_eq!(optional.get("temperature"), Some(&json!(0.5)));

    let mut non_default = object(json!({"temperature": 0.5}));
    let mut optional = object(json!({}));
    let failure = map_gpt5(
        &mut fixture,
        "gpt-5.5",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect_err("a declared default of high forbids a temperature");
    assert_eq!(failure.status_code(), 400);

    let mut non_default = object(json!({"temperature": 0.5}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.5",
        &mut non_default,
        &mut optional,
        request_drop(),
    )
    .await
    .expect("drop_params removes a forbidden temperature");
    assert!(!non_default.contains_key("temperature"));
    assert!(!optional.contains_key("temperature"));

    let mut non_default = object(json!({"temperature": 0.5, "reasoning_effort": "none"}));
    let mut optional = object(json!({}));
    let failure = map_gpt5(
        &mut fixture,
        "gpt-5",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect_err("a model that cannot resolve to none rejects a temperature");
    assert_eq!(failure.status_code(), 400);

    let mut non_default = object(json!({}));
    let mut optional = object(json!({"temperature": 0.3}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("an existing optional temperature is untouched");
    assert_eq!(optional.get("temperature"), Some(&json!(0.3)));
}

pub(super) async fn sampling_with_none() {
    let entries = r#"{
        "gpt-5.1": {
            "litellm_provider": "openai",
            "supports_tool_choice": true,
            "supports_none_reasoning_effort": true,
            "default_reasoning_effort": "none"
        },
        "gpt-5": {
            "litellm_provider": "openai",
            "supports_tool_choice": true,
            "supports_none_reasoning_effort": false
        }
    }"#;
    let mut fixture = Fixture::new(&["gpt-5.1", "gpt-5"], entries);
    let mut non_default =
        object(json!({"logprobs": true, "top_p": 0.9, "reasoning_effort": "high"}));
    let mut optional = object(json!({}));
    let failure = map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect_err("sampling needs effort none");
    assert_eq!(failure.status_code(), 400);

    let mut non_default =
        object(json!({"logprobs": true, "top_p": 0.9, "reasoning_effort": "high"}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        request_drop(),
    )
    .await
    .expect("drop_params removes the sampling params");
    assert!(!non_default.contains_key("logprobs"));
    assert!(!non_default.contains_key("top_p"));
    assert!(!non_default.contains_key("top_logprobs"));
    assert!(!optional.contains_key("logprobs"));

    let mut non_default = object(json!({"logprobs": true, "reasoning_effort": "none"}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("an explicit none effort permits sampling");
    assert_eq!(optional.get("logprobs"), Some(&json!(true)));

    let mut non_default = object(json!({"top_p": 0.5}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a declared default of none permits sampling");
    assert_eq!(optional.get("top_p"), Some(&json!(0.5)));

    let mut non_default = object(json!({"logprobs": true, "top_p": 0.5}));
    let mut optional = object(json!({}));
    map_gpt5(
        &mut fixture,
        "gpt-5",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("a model without none support never enters the sampling guard");
    assert!(!optional.contains_key("logprobs"));
    assert!(!optional.contains_key("top_p"));
    assert_eq!(non_default.get("logprobs"), Some(&json!(true)));

    let mut non_default = object(json!({}));
    let mut optional = object(json!({"logprobs": true, "top_p": 0.4}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("existing optional sampling values are not popped");
    assert_eq!(optional.get("logprobs"), Some(&json!(true)));
    assert_eq!(optional.get("top_p"), Some(&json!(0.4)));

    let mut non_default = object(json!({}));
    let mut optional = object(json!({"reasoning_effort": "high", "logprobs": true}));
    map_gpt5(
        &mut fixture,
        "gpt-5.1",
        &mut non_default,
        &mut optional,
        no_drop(),
    )
    .await
    .expect("the sampling branch reads only non_default_params");
    assert_eq!(optional.get("logprobs"), Some(&json!(true)));
}

pub(super) async fn o1_developer_roles() {
    let messages: Vec<crate::chat::ChatMessage> = serde_json::from_value(json!([
        {"role": "system", "content": "be brief", "name": "instruction"},
        {"role": "developer", "content": "use the tools"},
        {"role": "user", "content": "hello"}
    ]))
    .expect("messages");
    let mut fixture = Fixture::new(
        &["o3"],
        r#"{"o3": {"litellm_provider": "openai", "supports_system_messages": false}}"#,
    );
    let first = o_series::transform_messages(&mut fixture, "o3", &messages).await;
    let second = o_series::transform_messages(&mut fixture, "o3", &messages).await;
    assert_eq!(first, second, "one transform serves both paths");
    assert_eq!(first[0].role, crate::chat::Role::User);
    assert_eq!(
        first[0].content,
        Some(crate::chat::MessageContent::Text(String::from("be brief")))
    );
    assert_eq!(first[0].name, None);
    assert_eq!(first[1].role, crate::chat::Role::Developer);
    assert_eq!(first[1].content, messages[1].content);
    assert_eq!(first[2], messages[2]);

    let mut fixture = Fixture::new(
        &["o3"],
        r#"{"o3": {"litellm_provider": "openai", "supports_system_messages": true}}"#,
    );
    let kept = o_series::transform_messages(&mut fixture, "o3", &messages).await;
    assert_eq!(kept, messages);
}

pub(super) fn audio_token_parameter() {
    let fixture = Fixture::new(
        &["gpt-audio"],
        r#"{"gpt-audio": {"litellm_provider": "openai", "supports_tool_choice": true, "supports_function_calling": true}}"#,
    );
    let supported = audio::supported_params("gpt-audio", &fixture.catalog);
    assert_eq!(
        supported.iter().filter(|param| **param == "audio").count(),
        2
    );
    assert!(supported.contains(&"max_tokens") && supported.contains(&"user"));
    assert!(!supported.contains(&"reasoning_effort"));
    let mut non_default = object(json!({
        "max_tokens": 7,
        "audio": {"voice": "alloy"},
        "temperature": 0.3,
        "reasoning_effort": "high"
    }));
    let mut optional = object(json!({"existing": true}));
    audio::map_params(
        "gpt-audio",
        &mut non_default,
        &mut optional,
        &fixture.catalog,
    );
    assert_eq!(optional.get("max_tokens"), Some(&json!(7)));
    assert!(!optional.contains_key("max_completion_tokens"));
    assert_eq!(optional.get("audio"), Some(&json!({"voice": "alloy"})));
    assert_eq!(optional.get("temperature"), Some(&json!(0.3)));
    assert_eq!(optional.get("existing"), Some(&json!(true)));
    assert!(!optional.contains_key("reasoning_effort"));
}

pub(super) async fn gpt5_regular_allowlist() {
    let entries = r#"{
        "gpt-5.1": {
            "litellm_provider": "openai",
            "supports_tool_choice": true,
            "supports_none_reasoning_effort": true,
            "default_reasoning_effort": "none"
        },
        "gpt-5": {
            "litellm_provider": "openai",
            "supports_tool_choice": false,
            "supports_none_reasoning_effort": false
        }
    }"#;
    let mut fixture = Fixture::new(&["gpt-5.1", "gpt-5"], entries);
    let supported = gpt5_supported(&mut fixture, "gpt-5.1").await;
    assert_eq!(
        supported
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
        assert!(supported.contains(&present), "{present} should be allowed");
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
        assert!(!supported.contains(&absent), "{absent} should be omitted");
    }

    let supported = gpt5_supported(&mut fixture, "gpt-5").await;
    assert!(!supported.contains(&"tool_choice"));
    assert!(!supported.contains(&"logprobs"));
    assert!(!supported.contains(&"top_p"));
    assert!(!supported.contains(&"top_logprobs"));
    assert!(supported.contains(&"reasoning_effort") && supported.contains(&"user"));

    let supported = gpt5_supported(&mut fixture, "custom-gpt-5").await;
    assert!(!supported.contains(&"user"));
    assert!(!supported.contains(&"tool_choice"));
}

pub(super) fn version_boundaries() {
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
    assert_eq!(gpt_series_version("gpt-5.4x"), None);
    assert_eq!(gpt_series_version("gpt-5x"), None);
    assert_eq!(gpt_series_version("GPT-5.4"), None);
    assert_eq!(gpt_series_version("gpt-"), None);
    assert_eq!(gpt_series_version("gpt-5/"), None);
    assert!(!is_model_gpt_5_4_plus_model("gpt-5.3"));
    assert!(is_model_gpt_5_4_plus_model("gpt-5.4"));
    assert!(is_model_gpt_5_4_plus_model("gpt-5.39"));
    assert!(!is_model_gpt_5_6_plus_model("gpt-5.5"));
    assert!(is_model_gpt_5_6_plus_model("gpt-5.6"));
    assert!(is_model_gpt_5_6_plus_model("gpt-6.0"));
    assert!(!is_model_gpt_6_plus_model("gpt-5.6"));
    assert!(is_model_gpt_6_plus_model("gpt-6"));
    assert!(is_model_gpt_5_2_model("gpt-5.2"));
    assert!(is_model_gpt_5_2_model("gpt-5.2-pro"));
    assert!(is_model_gpt_5_2_model("gpt-5.40"));
    assert!(!is_model_gpt_5_2_model("gpt-5.3"));
    assert!(!is_model_gpt_5_2_model("gpt-6"));
    assert!(!is_model_gpt_5_4_plus_model("gpt-4.9"));
}
