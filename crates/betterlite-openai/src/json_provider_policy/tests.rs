#![cfg(test)]

use std::collections::BTreeMap;

use betterlite_config::sections::openai_like_settings::Settings as OpenAiLikeSettings;
use betterlite_models::identity::provider_id::ProviderId;
use betterlite_models::json_provider_registry::{
    BaseClass, Constraints, JSONProviderRegistry, ProviderConfig, ProviderDescriptor,
};
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::responses::{OpenAiLikeResponsesPolicy, ResponsesPolicy, ResponsesPolicyCache};
use super::{ChatCapabilities, ChatParent, ChatPolicy, TOOL_PARAMS, base_class_parent};
use crate::chat::ChatMessage;
use crate::openai_base_config::Headers;
use crate::response_input::ResponsesInput;

const KEY_ENV: &str = "PROBE_API_KEY";
const BASE_ENV: &str = "PROBE_API_BASE";

fn config(base_url: &str, base_class: &str) -> ProviderConfig {
    ProviderConfig {
        base_url: base_url.to_owned(),
        api_key_env: KEY_ENV.to_owned(),
        api_base_env: Some(BASE_ENV.to_owned()),
        base_class: BaseClass(base_class.to_owned()),
        param_mappings: BTreeMap::new(),
        constraints: Constraints::default(),
        special_handling: BTreeMap::new(),
        supported_endpoints: Vec::new(),
    }
}

fn chat_policy(descriptor: ProviderDescriptor) -> ChatPolicy {
    ChatPolicy { descriptor }
}

fn responses_policy_from(descriptor: ProviderDescriptor) -> ResponsesPolicy {
    ResponsesPolicy { descriptor }
}

fn chat(base_url: &str) -> ChatPolicy {
    chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: config(base_url, "openai_like"),
    })
}

fn responses(base_url: &str) -> ResponsesPolicy {
    responses_policy_from(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: config(base_url, "openai_like"),
    })
}

fn environment(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn object(pairs: &[(&str, Json)]) -> Object {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), value.clone()))
        .collect()
}

fn text(value: &str) -> Json {
    Json::Text(value.to_owned())
}

fn number(value: i64) -> Json {
    Json::Number(serde_json::Number::from(value))
}

fn decimal(value: &str) -> Json {
    serde_json::from_str(value).expect("a JSON number")
}

#[rstest]
#[case::gpt("openai_gpt", ChatParent::Gpt)]
#[case::like("openai_like", ChatParent::Compatible)]
#[case::empty("", ChatParent::Compatible)]
#[case::other("custom", ChatParent::Compatible)]
fn parent_follows_the_base_class(#[case] base_class: &str, #[case] parent: ChatParent) {
    assert_eq!(
        base_class_parent(
            &chat_policy(ProviderDescriptor {
                provider: ProviderId::Cortecs,
                config: config("https://example.invalid/v1", base_class),
            })
            .descriptor
            .config
            .base_class
        ),
        parent
    );
}

#[test]
fn pinned_publicai_and_sail_descriptors_drive_the_policy() {
    let registry = JSONProviderRegistry::load();
    let publicai = registry
        .providers
        .get(&ProviderId::Publicai)
        .cloned()
        .map(|descriptor| ChatPolicy { descriptor })
        .expect("publicai");
    assert!(publicai.converts_content_lists());
    assert_eq!(
        base_class_parent(&publicai.descriptor.config.base_class),
        ChatParent::Gpt
    );
    assert_eq!(
        publicai.complete_url(None),
        "https://api.publicai.co/v1/chat/completions"
    );
    let sail = registry
        .providers
        .get(&ProviderId::Sail)
        .cloned()
        .map(|descriptor| ChatPolicy { descriptor })
        .expect("sail");
    assert!(!sail.converts_content_lists());
    assert_eq!(
        base_class_parent(&sail.descriptor.config.base_class),
        ChatParent::Gpt
    );
    assert!(registry.providers.get(&ProviderId::Openai).is_none());
}

#[test]
fn policy_accessors_and_lookup_report_the_descriptor() {
    let policy = chat("https://example.invalid/v1");
    assert_eq!(policy.descriptor.provider, ProviderId::Cortecs);
    assert_eq!(
        policy.descriptor.config.base_url.as_str(),
        "https://example.invalid/v1"
    );
    let policy = responses("https://descriptor.invalid/v1");
    assert_eq!(policy.descriptor.provider, ProviderId::Cortecs);
    assert_eq!(
        policy.descriptor.config.base_url.as_str(),
        "https://descriptor.invalid/v1"
    );
    let registry = JSONProviderRegistry::load();
    let from_registry = registry
        .providers
        .get(&ProviderId::Parasail)
        .cloned()
        .map(|descriptor| ResponsesPolicy { descriptor })
        .expect("parasail is pinned");
    assert_eq!(from_registry.descriptor.provider, ProviderId::Parasail);
    assert!(registry.providers.get(&ProviderId::Openai).is_none());
}

#[test]
fn identity_prefers_the_explicit_base_then_the_declared_environment() {
    let policy = chat("https://descriptor.invalid/v1");
    let explicit_key = SecretString::from("sk-explicit");
    let lookup = environment(&[(KEY_ENV, "sk-env"), (BASE_ENV, "https://env.invalid/v1")]);
    let (base, key) = policy.resolve_provider_info(
        Some("https://explicit.invalid/v1"),
        Some(&explicit_key),
        &lookup,
    );
    assert_eq!(base, "https://explicit.invalid/v1");
    assert_eq!(
        key.as_ref().map(|secret| secret.expose_secret()),
        Some("sk-explicit")
    );
    let (base, key) = policy.resolve_provider_info(Some(""), None, &lookup);
    assert_eq!(base, "https://env.invalid/v1");
    assert_eq!(
        key.as_ref().map(|secret| secret.expose_secret()),
        Some("sk-env")
    );
    let (base, key) = policy.resolve_provider_info(None, None, &environment(&[(KEY_ENV, "")]));
    assert_eq!(base, "https://descriptor.invalid/v1");
    assert!(key.is_none());
    let (base, _) = policy.resolve_provider_info(None, None, &environment(&[(BASE_ENV, "")]));
    assert_eq!(base, "https://descriptor.invalid/v1");
}

#[test]
fn chat_url_appends_the_suffix_without_slash_cleanup() {
    let policy = chat("https://descriptor.invalid/v1");
    assert_eq!(
        policy.complete_url(Some("https://explicit.invalid/v1")),
        "https://explicit.invalid/v1/chat/completions"
    );
    assert_eq!(
        policy.complete_url(Some("https://explicit.invalid/v1/chat/completions")),
        "https://explicit.invalid/v1/chat/completions"
    );
    assert_eq!(
        policy.complete_url(Some("https://explicit.invalid/v1/")),
        "https://explicit.invalid/v1//chat/completions"
    );
    assert_eq!(
        policy.complete_url(None),
        "https://descriptor.invalid/v1/chat/completions"
    );
    assert_eq!(
        policy.complete_url(Some("")),
        "https://descriptor.invalid/v1/chat/completions"
    );
}

#[rstest]
#[case::none(ChatCapabilities::default(), false, false)]
#[case::function_calling(
    ChatCapabilities {
        supports_function_calling: true,
        supports_reasoning: false,
    },
    true,
    false
)]
#[case::reasoning(
    ChatCapabilities {
        supports_function_calling: false,
        supports_reasoning: true,
    },
    false,
    true
)]
#[case::both(
    ChatCapabilities {
        supports_function_calling: true,
        supports_reasoning: true,
    },
    true,
    true
)]
fn capability_filtering_removes_tools_and_adds_reasoning_effort(
    #[case] capabilities: ChatCapabilities,
    #[case] keeps_tools: bool,
    #[case] adds_reasoning_effort: bool,
) {
    let policy = chat("https://example.invalid/v1");
    let mut parent = vec!["temperature", "tools", "tools_choice", "max_tokens"];
    parent.extend(TOOL_PARAMS);
    let supported = policy.supported_params(&parent, capabilities);
    assert_eq!(
        supported.iter().any(|param| param == "parallel_tool_calls"),
        keeps_tools
    );
    assert_eq!(
        supported.iter().any(|param| param == "reasoning_effort"),
        adds_reasoning_effort
    );
    assert!(supported.iter().any(|param| param == "temperature"));
    assert!(supported.iter().any(|param| param == "max_tokens"));
    assert!(supported.iter().any(|param| param == "tools_choice"));
}

#[test]
fn reasoning_effort_is_not_appended_twice() {
    let policy = chat("https://example.invalid/v1");
    let supported = policy.supported_params(
        &["reasoning_effort"],
        ChatCapabilities {
            supports_function_calling: true,
            supports_reasoning: true,
        },
    );
    assert_eq!(supported, vec!["reasoning_effort".to_owned()]);
}

#[test]
fn mapping_wins_before_the_supported_check() {
    let mut provider = config("https://example.invalid/v1", "openai_gpt");
    provider
        .param_mappings
        .insert("max_completion_tokens".to_owned(), "max_tokens".to_owned());
    provider
        .param_mappings
        .insert("tools".to_owned(), "mapped_tools".to_owned());
    let policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: provider,
    });
    let non_default = object(&[
        ("max_completion_tokens", number(16)),
        ("tools", Json::List(Vec::new())),
        ("tool_choice", text("auto")),
        ("unsupported", number(1)),
    ]);
    let mut existing = object(&[("temperature", number(2))]);
    policy
        .map_params(&non_default, &mut existing, &["tool_choice".to_owned()])
        .expect("the mapped and supported params are kept");
    assert_eq!(
        existing.get("max_tokens"),
        Some(&number(16)),
        "a mapping replaces the caller's parameter name even when the supported list lacks its target"
    );
    assert_eq!(existing.get("mapped_tools"), Some(&Json::List(Vec::new())));
    assert_eq!(existing.get("tool_choice"), Some(&text("auto")));
    assert!(!existing.contains_key("max_completion_tokens"));
    assert!(!existing.contains_key("tools"));
    assert!(!existing.contains_key("unsupported"));
}

fn constrained(provider: &mut ProviderConfig) {
    provider.constraints.temperature_max = Some("0.5".parse().expect("decimal"));
    provider.constraints.temperature_min = Some("0.7".parse().expect("decimal"));
    provider.constraints.temperature_min_with_n_gt_1 = Some("0.9".parse().expect("decimal"));
}

#[rstest]
#[case::the_min_follows_the_max("5", Some(4), "0.9")]
#[case::the_n_step_runs_last("2", Some(3), "0.9")]
#[case::n_absent_means_one("0.9", None, "0.7")]
#[case::n_equal_to_one_skips_the_step("0.1", Some(1), "0.7")]
fn temperature_clamps_run_in_max_min_then_n_order(
    #[case] temperature: &str,
    #[case] n: Option<i64>,
    #[case] expected: &str,
) {
    let mut provider = config("https://example.invalid/v1", "openai_like");
    constrained(&mut provider);
    let policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: provider,
    });
    let mut existing = object(&[("temperature", decimal(temperature))]);
    if let Some(n) = n {
        existing.insert("n".to_owned(), number(n));
    }
    policy
        .map_params(&Object::new(), &mut existing, &[])
        .expect("numbers clamp");
    assert_eq!(existing.get("temperature"), Some(&decimal(expected)));
}

#[test]
fn a_temperature_at_every_limit_keeps_the_callers_number() {
    let mut provider = config("https://example.invalid/v1", "openai_like");
    provider.constraints.temperature_max = Some("1000".parse().expect("decimal"));
    provider.constraints.temperature_min = Some("1000".parse().expect("decimal"));
    provider.constraints.temperature_min_with_n_gt_1 = Some("1000".parse().expect("decimal"));
    let policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: provider,
    });
    let mut existing = object(&[("temperature", decimal("1000.0")), ("n", number(2))]);
    policy
        .map_params(&Object::new(), &mut existing, &[])
        .expect("numbers clamp");
    assert_eq!(existing.get("temperature"), Some(&decimal("1000.0")));
}

#[test]
fn the_n_step_can_exceed_the_max_it_ran_before() {
    let mut provider = config("https://example.invalid/v1", "openai_like");
    provider.constraints.temperature_max = Some("0.5".parse().expect("decimal"));
    provider.constraints.temperature_min = Some("0.2".parse().expect("decimal"));
    provider.constraints.temperature_min_with_n_gt_1 = Some("0.9".parse().expect("decimal"));
    let policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: provider,
    });
    let mut existing = object(&[("temperature", decimal("0.4")), ("n", number(4))]);
    policy
        .map_params(&Object::new(), &mut existing, &[])
        .expect("numbers clamp");
    assert_eq!(existing.get("temperature"), Some(&decimal("0.9")));
}

#[test]
fn a_temperature_at_or_above_the_n_limit_stays() {
    let mut provider = config("https://example.invalid/v1", "openai_like");
    provider.constraints.temperature_min_with_n_gt_1 = Some("0.9".parse().expect("decimal"));
    let policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: provider,
    });
    let mut above = object(&[("temperature", decimal("0.95")), ("n", number(4))]);
    policy
        .map_params(&Object::new(), &mut above, &[])
        .expect("numbers clamp");
    assert_eq!(above.get("temperature"), Some(&decimal("0.95")));
    let mut at = object(&[("temperature", decimal("0.9")), ("n", number(4))]);
    policy
        .map_params(&Object::new(), &mut at, &[])
        .expect("numbers clamp");
    assert_eq!(at.get("temperature"), Some(&decimal("0.9")));
}

#[test]
fn null_temperature_is_an_error_only_when_a_comparison_runs() {
    let mut provider = config("https://example.invalid/v1", "openai_like");
    provider.constraints.temperature_min_with_n_gt_1 = Some("0.5".parse().expect("decimal"));
    let policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: provider,
    });
    let mut null_temperature = object(&[("temperature", Json::Null)]);
    policy
        .map_params(&Object::new(), &mut null_temperature, &[])
        .expect("n defaults to one, so Python never compares the null");
    assert_eq!(null_temperature.get("temperature"), Some(&Json::Null));
    let mut with_n = object(&[("temperature", Json::Null), ("n", number(2))]);
    assert!(policy.map_params(&Object::new(), &mut with_n, &[]).is_err());
    let mut only_max = config("https://example.invalid/v1", "openai_like");
    only_max.constraints.temperature_max = Some("0.5".parse().expect("decimal"));
    let max_policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: only_max,
    });
    let mut null_with_max = object(&[("temperature", Json::Null)]);
    assert!(
        max_policy
            .map_params(&Object::new(), &mut null_with_max, &[])
            .is_err()
    );
}

#[test]
fn null_n_is_an_error_when_the_n_constraint_runs() {
    let mut provider = config("https://example.invalid/v1", "openai_like");
    provider.constraints.temperature_min_with_n_gt_1 = Some("0.5".parse().expect("decimal"));
    let policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Cortecs,
        config: provider,
    });
    let mut existing = object(&[("temperature", decimal("0.1")), ("n", Json::Null)]);
    assert!(
        policy
            .map_params(&Object::new(), &mut existing, &[])
            .is_err()
    );
}

fn message(value: Json) -> ChatMessage {
    serde_json::from_value(value).expect("a chat message")
}

#[test]
fn transform_messages_collapses_only_when_the_descriptor_asks() {
    let message_json = serde_json::json!({
        "role": "user",
        "content": [
            {"type": "text", "text": "hello"},
            {"type": "text", "text": " world"},
        ],
        "name": "probe",
    });
    let messages = [message(message_json.clone())];
    let collapsing = chat_policy(ProviderDescriptor {
        provider: ProviderId::Publicai,
        config: ProviderConfig {
            special_handling: BTreeMap::from([("convert_content_list_to_string".to_owned(), true)]),
            ..config("https://example.invalid/v1", "openai_gpt")
        },
    });
    let collapsed = collapsing
        .transform_messages(&messages)
        .expect("the messages collapse");
    assert_eq!(
        serde_json::to_value(&collapsed).expect("serialized"),
        serde_json::json!([{"role": "user", "content": "hello world", "name": "probe"}])
    );
    let unchanged = chat("https://example.invalid/v1")
        .transform_messages(&messages)
        .expect("the messages pass through");
    assert_eq!(
        serde_json::to_value(&unchanged).expect("serialized"),
        serde_json::json!([message_json])
    );
}

#[test]
fn empty_content_lists_stay_lists() {
    let policy = chat_policy(ProviderDescriptor {
        provider: ProviderId::Publicai,
        config: ProviderConfig {
            special_handling: BTreeMap::from([("convert_content_list_to_string".to_owned(), true)]),
            ..config("https://example.invalid/v1", "openai_gpt")
        },
    });
    let messages = [message(serde_json::json!({"role": "user", "content": []}))];
    let transformed = policy.transform_messages(&messages).expect("messages");
    assert_eq!(
        serde_json::to_value(&transformed).expect("serialized"),
        serde_json::json!([{"role": "user", "content": []}])
    );
}

#[test]
fn responses_url_trims_every_slash_and_always_appends() {
    let policy = responses("https://descriptor.invalid/v1");
    let lookup = environment(&[(BASE_ENV, "https://env.invalid/v1/")]);
    assert_eq!(
        policy.complete_url(Some("https://explicit.invalid/v1/"), &lookup),
        "https://explicit.invalid/v1/responses"
    );
    assert_eq!(
        policy.complete_url(Some("https://explicit.invalid/v1///"), &lookup),
        "https://explicit.invalid/v1/responses"
    );
    assert_eq!(
        policy.complete_url(Some("https://explicit.invalid/v1/responses"), &lookup),
        "https://explicit.invalid/v1/responses/responses"
    );
    assert_eq!(
        policy.complete_url(None, &lookup),
        "https://env.invalid/v1/responses"
    );
    assert_eq!(
        policy.complete_url(None, &environment(&[])),
        "https://descriptor.invalid/v1/responses"
    );
}

#[test]
fn responses_authorization_is_written_only_for_a_truthy_key() {
    let policy = responses("https://descriptor.invalid/v1");
    let headers = Headers::from([("Content-Type".to_owned(), "application/json".to_owned())]);
    let explicit = SecretString::from("sk-explicit");
    let with_key = policy.validate_environment(headers.clone(), Some(&explicit), &environment(&[]));
    assert_eq!(
        with_key.get("Authorization").map(String::as_str),
        Some("Bearer sk-explicit")
    );
    assert_eq!(
        with_key.get("Content-Type").map(String::as_str),
        Some("application/json")
    );
    let without_key = policy.validate_environment(
        headers.clone(),
        Some(&SecretString::from("")),
        &environment(&[(KEY_ENV, "")]),
    );
    assert!(!without_key.contains_key("Authorization"));
    let from_environment =
        policy.validate_environment(headers, None, &environment(&[(KEY_ENV, "sk-env")]));
    assert_eq!(
        from_environment.get("Authorization").map(String::as_str),
        Some("Bearer sk-env")
    );
}

#[test]
fn responses_store_follows_the_force_store_flag() {
    let mut provider = config("https://descriptor.invalid/v1", "openai_gpt");
    provider
        .special_handling
        .insert("force_store_false".to_owned(), true);
    let forced = responses_policy_from(ProviderDescriptor {
        provider: ProviderId::Parasail,
        config: provider,
    });
    let params = object(&[("store", Json::Bool(true)), ("temperature", number(1))]);
    let body = forced
        .transform_request(
            "some-model",
            &ResponsesInput::Text("hi".to_owned()),
            &params,
            None,
        )
        .expect("the request builds");
    assert_eq!(body.get("store"), Some(&Json::Bool(false)));
    assert_eq!(body.get("temperature"), Some(&number(1)));
    let plain = responses("https://descriptor.invalid/v1");
    let body = plain
        .transform_request(
            "some-model",
            &ResponsesInput::Text("hi".to_owned()),
            &params,
            None,
        )
        .expect("the request builds");
    assert_eq!(body.get("store"), Some(&Json::Bool(true)));
}

#[test]
fn the_responses_cache_keeps_the_first_descriptor_for_a_slug() {
    let mut cache = ResponsesPolicyCache::default();
    assert!(cache.policies.is_empty());
    let first = ProviderDescriptor {
        provider: ProviderId::Parasail,
        config: config("https://first.invalid/v1", "openai_gpt"),
    };
    let second = ProviderDescriptor {
        provider: ProviderId::Parasail,
        config: config("https://second.invalid/v1", "openai_gpt"),
    };
    assert_eq!(
        cache
            .policy_for_descriptor(first)
            .complete_url(None, &environment(&[])),
        "https://first.invalid/v1/responses"
    );
    assert_eq!(
        cache
            .policy_for_descriptor(second)
            .complete_url(None, &environment(&[])),
        "https://first.invalid/v1/responses"
    );
    assert_eq!(cache.policies.len(), 1);
    assert!(!cache.policies.is_empty());
    let other = ProviderDescriptor {
        provider: ProviderId::Edenai,
        config: config("https://edenai.invalid/v1", "openai_gpt"),
    };
    assert_eq!(
        cache
            .policy_for_descriptor(other)
            .complete_url(None, &environment(&[])),
        "https://edenai.invalid/v1/responses"
    );
    assert_eq!(cache.policies.len(), 2);
}

#[test]
fn the_responses_cache_reads_the_registry_by_provider() {
    let registry = JSONProviderRegistry::load();
    let mut cache = ResponsesPolicyCache::default();
    let policy = cache
        .policy(&registry, ProviderId::Parasail)
        .expect("parasail is pinned");
    assert_eq!(policy.descriptor.provider, ProviderId::Parasail);
    assert!(
        cache.policy(&registry, ProviderId::Openai).is_none(),
        "a provider without a descriptor is never cached"
    );
    assert_eq!(cache.policies.len(), 1);
}

#[rstest]
#[case::explicit(
    Some("https://explicit.invalid/v1"),
    Some("https://env.invalid/v1"),
    Ok("https://explicit.invalid/v1/responses")
)]
#[case::environment(
    None,
    Some("https://env.invalid/v1/"),
    Ok("https://env.invalid/v1/responses")
)]
#[case::absent(None, None, Err(()))]
fn openai_like_responses_requires_a_base(
    #[case] explicit: Option<&str>,
    #[case] environment_base: Option<&str>,
    #[case] expected: Result<&str, ()>,
) {
    let mut settings = OpenAiLikeSettings::default();
    settings.api_base = environment_base.map(str::to_owned);
    let policy = OpenAiLikeResponsesPolicy { settings };
    match expected {
        Ok(url) => assert_eq!(policy.complete_url(explicit).expect("a base"), url),
        Err(()) => assert_eq!(
            policy
                .complete_url(explicit)
                .expect_err("the generic provider has no default base")
                .to_string(),
            "api_base is required for openai_like provider"
        ),
    }
}

#[test]
fn openai_like_responses_preserves_headers_for_a_truthy_key() {
    let mut settings = OpenAiLikeSettings::default();
    settings.api_key = Some(SecretString::from("sk-env"));
    let policy = OpenAiLikeResponsesPolicy { settings };
    assert!(policy.settings.api_key.is_some());
    let headers = Headers::from([("X-Trace".to_owned(), "1".to_owned())]);
    let with_environment = policy.validate_environment(headers.clone(), None);
    assert_eq!(
        with_environment.get("Authorization").map(String::as_str),
        Some("Bearer sk-env")
    );
    assert_eq!(
        with_environment.get("X-Trace").map(String::as_str),
        Some("1")
    );
    let without_key = OpenAiLikeResponsesPolicy {
        settings: OpenAiLikeSettings::default(),
    }
    .validate_environment(headers, Some(&SecretString::from("")));
    assert!(!without_key.contains_key("Authorization"));
}
