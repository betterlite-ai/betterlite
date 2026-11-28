#![cfg(test)]

use std::num::ParseIntError;
use std::panic::Location;
use std::path::Path;

use rstest::rstest;

use crate::bug_report::environment::Host;
use crate::bug_report::providers::KNOWN_PROVIDERS;
use crate::bug_report::record::Surface;
use crate::bug_report::report::exception_type_name;
use crate::safe_proxy_config::credential::{CREDENTIAL_KEY_PARTS, is_credential_key};
use crate::safe_proxy_config::known_values::KNOWN_VALUES;
use crate::safe_proxy_config::litellm_settings::LITELLM_SETTINGS_KEYS;
use crate::safe_proxy_config::render::render_value;
use crate::safe_proxy_config::tables::{
    CACHE_PARAMS_KEYS, GENERAL_SETTINGS_KEYS, GUARDRAIL_KEYS, ROUTER_SETTINGS_KEYS,
};
use crate::safe_proxy_config::{
    ConfigObject, ConfigValue, ProxyCall, ProxyConfigSnapshot, build_proxy_bug_report,
    build_proxy_environment_report, safe_config_lines,
};

fn object(source: &str) -> ConfigObject {
    serde_json::from_str(source).unwrap()
}

fn lines(config: &str, general_settings: &str) -> Vec<String> {
    safe_config_lines(&object(config), &object(general_settings))
}

fn words(table: &str) -> Vec<&str> {
    table.split_ascii_whitespace().collect()
}

fn no_docker() -> Host<'static> {
    Host {
        dockerenv: Path::new("/missing"),
        python_version: None,
    }
}

#[track_caller]
fn failing_parse() -> (&'static Location<'static>, ParseIntError) {
    (Location::caller(), "x".parse::<u32>().unwrap_err())
}

fn snapshot() -> ProxyConfigSnapshot {
    ProxyConfigSnapshot {
        config: object(r#"{"litellm_settings": {"success_callback": ["openai"]}}"#),
        general_settings: object(r#"{"store_model_in_db": true}"#),
    }
}

#[test]
fn credential_key_parts_match_the_pinned_set() {
    assert_eq!(CREDENTIAL_KEY_PARTS.len(), 19);
}

#[rstest]
#[case::key("key")]
#[case::keys("keys")]
#[case::secret("secret")]
#[case::secrets("secrets")]
#[case::token("token")]
#[case::password("password")]
#[case::passwd("passwd")]
#[case::credential("credential")]
#[case::credentials("credentials")]
#[case::url("url")]
#[case::uri("uri")]
#[case::dsn("dsn")]
#[case::host("host")]
#[case::hosts("hosts")]
#[case::base("base")]
#[case::endpoint("endpoint")]
#[case::cert("cert")]
#[case::pem("pem")]
#[case::salt("salt")]
fn credential_key_every_part_makes_a_key_credential_like(#[case] part: &str) {
    assert!(is_credential_key(part), "{part}");
    assert!(
        is_credential_key(&format!("prefix_{part}_suffix")),
        "{part}"
    );
}

#[rstest]
#[case::hyphen("key-name")]
#[case::no_separator("apikey")]
#[case::plural("passwords")]
#[case::longer_word("keyboard")]
#[case::empty("")]
#[case::underscores("___")]
fn credential_key_other_shapes_are_not_credential_like(#[case] key: &str) {
    assert!(!is_credential_key(key), "{key}");
}

#[rstest]
#[case::exact("key_management_system", false)]
#[case::upper_case("KEY_MANAGEMENT_SYSTEM", true)]
#[case::plural("key_management_systems", true)]
fn credential_key_management_system_is_the_one_exception(
    #[case] key: &str,
    #[case] credential: bool,
) {
    assert_eq!(is_credential_key(key), credential, "{key}");
}

#[rstest]
#[case::credential_like_base("api_base")]
#[case::credential_like_host("redis_host")]
#[case::plain_setting("store_model_in_db")]
fn render_bool_under_any_key(#[case] key: &str) {
    assert_eq!(
        render_value(key, &ConfigValue::Bool(true)).as_deref(),
        Some("true"),
        "{key}"
    );
    assert_eq!(
        render_value(key, &ConfigValue::Bool(false)).as_deref(),
        Some("false"),
        "{key}"
    );
}

#[rstest]
#[case::provider("openai", true)]
#[case::routing_strategy("simple-shuffle", true)]
#[case::cache_type("redis", true)]
#[case::secret_manager("local", true)]
#[case::guardrail("aporia", true)]
#[case::event_hook("pre_call", true)]
#[case::callback("pagerduty", true)]
#[case::unknown("open-ai", false)]
#[case::empty("", false)]
fn render_string_needs_a_known_value(#[case] text: &str, #[case] renders: bool) {
    let value = ConfigValue::Text(text.to_owned());
    assert_eq!(
        render_value("success_callback", &value).is_some(),
        renders,
        "{text}"
    );
}

#[rstest]
#[case::api_key("api_key")]
#[case::database_url("database_url")]
#[case::redis_host("redis_host")]
#[case::master_key("master_key")]
fn render_string_is_dropped_under_a_credential_like_key(#[case] key: &str) {
    let value = ConfigValue::Text(String::from("openai"));
    assert_eq!(render_value(key, &value), None, "{key}");
}

#[test]
fn render_string_under_the_credential_exception_key_is_kept() {
    let value = ConfigValue::Text(String::from("local"));
    assert_eq!(
        render_value("key_management_system", &value).as_deref(),
        Some("local")
    );
}

#[test]
fn render_list_joins_the_rendering_items() {
    let items = ConfigValue::List(vec![
        ConfigValue::Text(String::from("openai")),
        ConfigValue::Text(String::from("not-known")),
        ConfigValue::Bool(true),
    ]);
    assert_eq!(
        render_value("success_callback", &items).as_deref(),
        Some("[openai, true]")
    );
}

#[test]
fn render_list_without_rendering_items_is_none() {
    assert_eq!(
        render_value("success_callback", &ConfigValue::List(Vec::new())),
        None
    );
    let unknown = ConfigValue::List(vec![ConfigValue::Text(String::from("not-known"))]);
    assert_eq!(render_value("success_callback", &unknown), None);
    let numbers = ConfigValue::List(vec![ConfigValue::Number(serde_json::Number::from(7_u64))]);
    assert_eq!(render_value("success_callback", &numbers), None);
}

#[test]
fn render_nested_list_is_one_bracketed_item() {
    let nested = ConfigValue::List(vec![ConfigValue::List(vec![
        ConfigValue::Text(String::from("simple-shuffle")),
        ConfigValue::Bool(false),
    ])]);
    assert_eq!(
        render_value("routing_strategy_args", &nested).as_deref(),
        Some("[[simple-shuffle, false]]")
    );
}

#[test]
fn render_list_under_a_credential_like_key_drops_its_strings() {
    let items = ConfigValue::List(vec![
        ConfigValue::Text(String::from("openai")),
        ConfigValue::Bool(true),
    ]);
    assert_eq!(render_value("api_base", &items).as_deref(), Some("[true]"));
}

#[test]
fn render_number_null_and_object_are_dropped() {
    let integer = ConfigValue::Number(serde_json::Number::from(5_u64));
    assert_eq!(render_value("alerting_threshold", &integer), None);
    let float = ConfigValue::Number(serde_json::Number::from_f64(1.5).unwrap());
    assert_eq!(render_value("alerting_threshold", &float), None);
    assert_eq!(render_value("alerting_threshold", &ConfigValue::Null), None);
    let empty = ConfigValue::Object(ConfigObject::new());
    assert_eq!(render_value("alerting_threshold", &empty), None);
}

#[test]
fn section_order_follows_litellm() {
    let config = r#"{
        "guardrails": [{"litellm_params": {"mode": "pre_call"}}],
        "litellm_settings": {"cache_params": {"type": "redis"}, "success_callback": ["openai"]},
        "model_list": [{"litellm_params": {"model": "openai/gpt-4"}}],
        "router_settings": {"routing_strategy": "simple-shuffle"}
    }"#;
    assert_eq!(
        lines(config, r#"{"store_model_in_db": true}"#),
        [
            "general_settings.store_model_in_db = true",
            "litellm_settings.success_callback = [openai]",
            "litellm_settings.cache_params.type = redis",
            "router_settings.routing_strategy = simple-shuffle",
            "guardrails[0].litellm_params.mode = pre_call",
            "model_list[*].provider = [openai]",
        ]
    );
}

#[test]
fn section_general_settings_come_from_the_argument() {
    let config = r#"{"general_settings": {"store_model_in_db": true}}"#;
    assert!(lines(config, "{}").is_empty());
    assert_eq!(
        lines("{}", r#"{"store_model_in_db": true}"#),
        ["general_settings.store_model_in_db = true"]
    );
}

#[test]
fn section_value_that_is_not_an_object_gives_no_lines() {
    let config = r#"{
        "guardrails": "nope",
        "litellm_settings": [],
        "model_list": {"litellm_params": {"model": "openai/gpt-4"}},
        "router_settings": 7
    }"#;
    assert!(lines(config, "{}").is_empty());
    assert!(lines("{}", "{}").is_empty());
    let empty =
        r#"{"guardrails": [], "litellm_settings": {}, "model_list": [], "router_settings": {}}"#;
    assert!(lines(empty, "{}").is_empty());
}

#[test]
fn section_guardrail_entries_keep_their_index() {
    let config = r#"{"guardrails": ["nope", {"litellm_params": {"mode": "pre_call"}}, 7]}"#;
    assert_eq!(
        lines(config, "{}"),
        ["guardrails[1].litellm_params.mode = pre_call"]
    );
    let params_not_an_object = r#"{
        "guardrails": [{"litellm_params": "nope"}, {"litellm_params": {"mode": "post_call"}}]
    }"#;
    assert_eq!(
        lines(params_not_an_object, "{}"),
        ["guardrails[1].litellm_params.mode = post_call"]
    );
}

#[test]
fn section_model_list_reports_sorted_distinct_known_providers() {
    let config = r#"{
        "model_list": [
            {"litellm_params": {"model": "openai/gpt-4"}},
            {"litellm_params": {"model": "azure/text-embedding-3-small"}},
            {"litellm_params": {"model": "openai/gpt-4o"}},
            {"litellm_params": {"model": "not-a-provider/model"}},
            {"litellm_params": {"model": "anthropic/claude"}},
            "nope",
            {"litellm_params": []},
            {"litellm_params": {"model": 7}}
        ]
    }"#;
    assert_eq!(
        lines(config, "{}"),
        ["model_list[*].provider = [anthropic, azure, openai]"]
    );
}

#[test]
fn section_model_list_without_a_known_provider_gives_no_line() {
    let config = r#"{
        "model_list": [
            {"litellm_params": {"model": "GPT-4/claude"}},
            {"litellm_params": {"model": "openai"}},
            {"litellm_params": {"model": "/claude"}},
            {}
        ]
    }"#;
    assert!(lines(config, "{}").is_empty());
}

#[test]
fn section_credential_like_keys_keep_only_booleans() {
    let general = r#"{"database_url": "openai", "master_key": true, "store_model_in_db": false}"#;
    assert_eq!(
        lines("{}", general),
        [
            "general_settings.master_key = true",
            "general_settings.store_model_in_db = false",
        ]
    );
    let router = r#"{
        "router_settings": {"redis_host": "openai", "redis_password": "openai", "set_verbose": true}
    }"#;
    assert_eq!(lines(router, "{}"), ["router_settings.set_verbose = true"]);
}

#[test]
fn section_unknown_keys_give_no_lines() {
    let config = r#"{
        "litellm_settings": {"not_a_litellm_key": true},
        "router_settings": {"not_a_router_key": true}
    }"#;
    assert!(lines(config, r#"{"not_a_general_key": true}"#).is_empty());
}

#[test]
fn section_lines_are_sorted_by_key() {
    let general =
        r#"{"store_model_in_db": true, "allow_cli_sso_verification_uri_complete": false}"#;
    assert_eq!(
        lines("{}", general),
        [
            "general_settings.allow_cli_sso_verification_uri_complete = false",
            "general_settings.store_model_in_db = true",
        ]
    );
}

#[test]
fn table_general_settings_matches_the_pinned_fields() {
    let general = words(GENERAL_SETTINGS_KEYS);
    assert_eq!(general.len(), 107);
    assert!(!general.contains(&"not_a_litellm_key"), "unknown key");
    assert!(!general.contains(&"cache_params"), "not a general setting");
}

#[rstest]
#[case::store_model_in_db("store_model_in_db")]
#[case::master_key("master_key")]
#[case::key_management_system("key_management_system")]
#[case::database_url("database_url")]
fn table_general_settings_has_each_pinned_field(#[case] name: &str) {
    assert!(words(GENERAL_SETTINGS_KEYS).contains(&name), "{name}");
}

#[test]
fn table_litellm_settings_matches_the_pinned_module_names() {
    let settings = words(LITELLM_SETTINGS_KEYS);
    assert_eq!(settings.len(), 607);
    assert!(
        !settings.contains(&"priority_reservation_settings"),
        "bound inside if TYPE_CHECKING"
    );
    assert!(!settings.contains(&"os"), "a plain import binds no name");
    assert!(
        !settings.contains(&"cache_params"),
        "not a module-level name"
    );
}

#[rstest]
#[case::wildcard("*")]
#[case::custom_llm("CustomLLM")]
#[case::async_success_callback("_async_success_callback")]
#[case::success_callback("success_callback")]
#[case::num_retries("num_retries")]
fn table_litellm_settings_has_each_pinned_module_name(#[case] name: &str) {
    assert!(words(LITELLM_SETTINGS_KEYS).contains(&name), "{name}");
}

#[test]
fn table_cache_params_matches_the_init_parameters() {
    let params = words(CACHE_PARAMS_KEYS);
    assert_eq!(params.len(), 47);
    assert!(!params.contains(&"self"), "self is not a setting");
    assert!(!params.contains(&"redis_host"), "a router setting");
}

#[rstest]
#[case::cache_type("type")]
#[case::host("host")]
#[case::password("password")]
#[case::kwargs("kwargs")]
#[case::semantic_index("redis_semantic_cache_index_name")]
fn table_cache_params_has_each_init_parameter(#[case] name: &str) {
    assert!(words(CACHE_PARAMS_KEYS).contains(&name), "{name}");
}

#[test]
fn table_router_settings_matches_the_init_parameters() {
    let settings = words(ROUTER_SETTINGS_KEYS);
    assert_eq!(settings.len(), 58);
    assert!(!settings.contains(&"self"), "self is not a setting");
    assert!(
        !settings.contains(&"kwargs"),
        "Router.__init__ has no **kwargs"
    );
    assert!(
        !settings.contains(&"redis_semantic_cache_index_name"),
        "a cache setting"
    );
}

#[rstest]
#[case::routing_strategy("routing_strategy")]
#[case::redis_host("redis_host")]
#[case::model_list("model_list")]
#[case::set_verbose("set_verbose")]
fn table_router_settings_has_each_init_parameter(#[case] name: &str) {
    assert!(words(ROUTER_SETTINGS_KEYS).contains(&name), "{name}");
}

#[test]
fn table_guardrails_include_mixin_fields() {
    let fields = words(GUARDRAIL_KEYS);
    assert_eq!(fields.len(), 160);
    assert!(
        !fields.contains(&"litellm_params"),
        "litellm_params wraps these fields"
    );
    assert!(!fields.contains(&"self"), "self is not a field");
}

#[rstest]
#[case::api_base("api_base")]
#[case::api_key("api_key")]
#[case::mode("mode")]
#[case::guardrail("guardrail")]
#[case::presidio_analyzer_api_base("presidio_analyzer_api_base")]
#[case::on_flagged_action("on_flagged_action")]
fn table_guardrails_field_is_listed(#[case] name: &str) {
    assert!(words(GUARDRAIL_KEYS).contains(&name), "{name}");
}

#[test]
fn table_known_values_join_every_source() {
    let values = words(KNOWN_VALUES);
    assert_eq!(values.len(), 307);
    assert!(!values.contains(&"not_a_known_value"), "unknown value");
}

#[rstest]
#[case::routing_strategy("simple-shuffle")]
#[case::provider("openai")]
#[case::callback("pagerduty")]
#[case::email_callback("smtp_email")]
#[case::logger("datadog")]
#[case::cache_type("redis")]
#[case::secret_manager("local")]
#[case::guardrail("aporia")]
#[case::event_hook("pre_call")]
fn table_known_values_include_each_source_value(#[case] value: &str) {
    assert!(words(KNOWN_VALUES).contains(&value), "{value}");
}

#[test]
fn table_known_values_are_unique_and_sorted() {
    let values = words(KNOWN_VALUES);
    let mut sorted = values.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(values, sorted);
}

#[test]
fn table_known_values_include_every_known_provider() {
    let values = words(KNOWN_VALUES);
    let missing = KNOWN_PROVIDERS
        .split_ascii_whitespace()
        .find(|provider| !values.contains(provider));
    assert!(missing.is_none(), "{missing:?}");
}

#[test]
fn report_environment_uses_the_proxy_surface_and_lines() {
    let environment = build_proxy_environment_report(&snapshot(), no_docker());
    assert_eq!(environment.surface, Surface::Proxy);
    assert_eq!(environment.deployment, None);
    assert_eq!(
        environment.config_lines,
        [
            "general_settings.store_model_in_db = true",
            "litellm_settings.success_callback = [openai]",
        ]
    );
}

#[test]
fn report_bug_keeps_the_proxy_surface_and_call_fields() {
    let ((location, error), call_line) = (failing_parse(), line!());
    let call = ProxyCall {
        call_type: Some(String::from("acompletion")),
        custom_llm_provider: Some(String::from("openai")),
        stream: Some(true),
    };
    let built = build_proxy_bug_report(
        exception_type_name(&error),
        location,
        call,
        &snapshot(),
        no_docker(),
    );
    assert_eq!(built.environment.surface, Surface::Proxy);
    assert_eq!(
        built.environment.config_lines,
        [
            "general_settings.store_model_in_db = true",
            "litellm_settings.success_callback = [openai]",
        ]
    );
    assert_eq!(built.exception_type, "ParseIntError");
    assert_eq!(
        (built.call_type, built.custom_llm_provider, built.stream),
        (
            Some(String::from("acompletion")),
            Some(String::from("openai")),
            Some(true)
        )
    );
    assert_eq!(built.litellm_frames, [format!("{}:{call_line}", file!())]);
}

#[test]
fn report_bug_defaults_the_call_fields_to_none() {
    let (location, error) = failing_parse();
    let built = build_proxy_bug_report(
        exception_type_name(&error),
        location,
        ProxyCall::default(),
        &snapshot(),
        no_docker(),
    );
    assert_eq!(
        (built.call_type, built.custom_llm_provider, built.stream),
        (None, None, None)
    );
}
