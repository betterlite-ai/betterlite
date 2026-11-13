#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};

use betterlite_config::sections::gdc_settings::{BoolOrString, CallSettings, GlobalSettings};
use betterlite_prompts::arguments::json::{Json, Object};
use rstest::rstest;
use serde_json::Value;

use crate::chat::ChatMessage;
use crate::chat_content::RefuseDownloads;
use crate::openai_base_config::{
    CatalogCapabilities, ConfigFamily, Headers, OpenAiConfig, OpenAiSettings,
};

use super::{
    GdcPolicyError, audience, caching_enabled, complete_url, effective_project,
    get_supported_openai_params, read_env_bool, resolve_location, resolve_project,
    transform_request, validate_path_id,
};

fn parsed(text: &str) -> Value {
    serde_json::from_str(text).unwrap()
}

fn call() -> CallSettings {
    CallSettings {
        vertex_project: None,
        vertex_ai_project: None,
        vertex_location: None,
        vertex_ai_location: None,
        ssl_verify: None,
        gdc_token_caching: None,
    }
}

fn global() -> GlobalSettings {
    GlobalSettings {
        api_base: None,
        gdc_api_base: None,
        vertex_project: None,
        vertex_ai_project: None,
        vertex_location: None,
        vertex_ai_location: None,
    }
}

fn located_call() -> CallSettings {
    let mut call = call();
    call.vertex_project = Some("project".to_owned());
    call.vertex_location = Some("location".to_owned());
    call
}

fn optional(value: Value) -> Object {
    serde_json::from_value(value).unwrap()
}

fn config() -> OpenAiConfig {
    OpenAiConfig {
        family: ConfigFamily::Other,
        api_key: None,
        headers: Headers::new(),
        settings: OpenAiSettings::default(),
    }
}

fn error_status(error: &GdcPolicyError) -> u16 {
    error.class().metadata().status_code
}

#[test]
fn vertex_parameters_lead_the_parent_option_list() {
    let catalog = CatalogCapabilities {
        chat_models: BTreeSet::new(),
        text_models: BTreeSet::new(),
    };
    let supported = get_supported_openai_params("gemini-2.5-pro", &config(), &catalog);
    let mut expected = vec!["vertex_project", "vertex_location"];
    expected.extend(config().supported_params("gemini-2.5-pro", &catalog));
    assert_eq!(supported, expected);
    assert_eq!(supported.len(), 33);
    assert!(!supported.contains(&"reasoning_effort"));
}

#[test]
fn a_catalog_model_adds_user_after_the_parent_list() {
    let catalog = CatalogCapabilities {
        chat_models: BTreeSet::from([String::from("gemini-2.5-pro")]),
        text_models: BTreeSet::new(),
    };
    let supported = get_supported_openai_params("gemini-2.5-pro", &config(), &catalog);
    assert_eq!(supported.len(), 34);
    assert_eq!(supported.last(), Some(&"user"));
}

struct VertexRow {
    call_project: Option<&'static str>,
    call_alias: Option<&'static str>,
    call_location: Option<&'static str>,
    call_location_alias: Option<&'static str>,
    global_project: Option<&'static str>,
    global_location: Option<&'static str>,
    request: Value,
    project: Option<&'static str>,
    location: Option<&'static str>,
}

#[rstest]
#[case::call_values(VertexRow {
    call_project: Some("call"),
    call_alias: Some("call-alias"),
    call_location: Some("call-location"),
    call_location_alias: Some("call-alias-location"),
    global_project: Some("global"),
    global_location: Some("global-location"),
    request: parsed(r#"{"vertex_project": "optional", "vertex_ai_project": "optional-alias", "vertex_location": "optional-location", "vertex_ai_location": "optional-alias-location"}"#),
    project: Some("call"),
    location: Some("call-location"),
})]
#[case::call_aliases(VertexRow {
    call_project: None,
    call_alias: Some("call-alias"),
    call_location: None,
    call_location_alias: Some("call-alias-location"),
    global_project: Some("global"),
    global_location: Some("global-location"),
    request: parsed(r#"{"vertex_project": "optional"}"#),
    project: Some("call-alias"),
    location: Some("call-alias-location"),
})]
#[case::globals(VertexRow {
    call_project: None,
    call_alias: None,
    call_location: None,
    call_location_alias: None,
    global_project: Some("global"),
    global_location: Some("global-location"),
    request: parsed(r#"{"vertex_project": "optional", "vertex_location": "optional-location"}"#),
    project: Some("global"),
    location: Some("global-location"),
})]
#[case::optional_values(VertexRow {
    call_project: None,
    call_alias: None,
    call_location: None,
    call_location_alias: None,
    global_project: None,
    global_location: None,
    request: parsed(r#"{"vertex_project": "optional", "vertex_ai_project": "optional-alias", "vertex_location": "optional-location", "vertex_ai_location": "optional-alias-location"}"#),
    project: Some("optional"),
    location: Some("optional-location"),
})]
#[case::optional_aliases(VertexRow {
    call_project: None,
    call_alias: None,
    call_location: None,
    call_location_alias: None,
    global_project: None,
    global_location: None,
    request: parsed(r#"{"vertex_ai_project": "optional-alias", "vertex_ai_location": "optional-alias-location"}"#),
    project: Some("optional-alias"),
    location: Some("optional-alias-location"),
})]
#[case::empty_values_are_skipped(VertexRow {
    call_project: Some(""),
    call_alias: None,
    call_location: Some(""),
    call_location_alias: None,
    global_project: Some(""),
    global_location: Some(""),
    request: parsed(r#"{"vertex_project": "optional", "vertex_location": "optional-location"}"#),
    project: Some("optional"),
    location: Some("optional-location"),
})]
#[case::nothing_anywhere(VertexRow {
    call_project: None,
    call_alias: None,
    call_location: None,
    call_location_alias: None,
    global_project: None,
    global_location: None,
    request: parsed("{}"),
    project: None,
    location: None,
})]
fn resolves_vertex_values_through_the_litellm_order(#[case] row: VertexRow) {
    let mut call = call();
    call.vertex_project = row.call_project.map(str::to_owned);
    call.vertex_ai_project = row.call_alias.map(str::to_owned);
    call.vertex_location = row.call_location.map(str::to_owned);
    call.vertex_ai_location = row.call_location_alias.map(str::to_owned);
    let mut global = global();
    global.vertex_project = row.global_project.map(str::to_owned);
    global.vertex_location = row.global_location.map(str::to_owned);
    let request = optional(row.request);
    let project = resolve_project(&call, &global, &request);
    let location = resolve_location(&call, &global, &request);
    assert_eq!(project.as_ref().and_then(Json::as_str), row.project);
    assert_eq!(location.as_ref().and_then(Json::as_str), row.location);
}

#[rstest]
#[case::number(
    parsed(r#"{"vertex_project": 5, "vertex_location": "location"}"#),
    true
)]
#[case::truth(
    parsed(r#"{"vertex_project": true, "vertex_location": "location"}"#),
    true
)]
#[case::list(
    parsed(r#"{"vertex_project": ["p"], "vertex_location": "location"}"#),
    true
)]
#[case::object(
    parsed(r#"{"vertex_project": {"name": "p"}, "vertex_location": "location"}"#),
    true
)]
#[case::zero(
    parsed(r#"{"vertex_project": 0, "vertex_location": "location"}"#),
    false
)]
#[case::false_value(
    parsed(r#"{"vertex_project": false, "vertex_location": "location"}"#),
    false
)]
#[case::null(
    parsed(r#"{"vertex_project": null, "vertex_location": "location"}"#),
    false
)]
#[case::empty_text(
    parsed(r#"{"vertex_project": "", "vertex_location": "location"}"#),
    false
)]
#[case::empty_list(
    parsed(r#"{"vertex_project": [], "vertex_location": "location"}"#),
    false
)]
fn a_truthy_non_string_project_is_a_parameter_error(
    #[case] request: Value,
    #[case] rejected: bool,
) {
    let outcome = complete_url(
        Some("https://gateway"),
        &call(),
        &global(),
        &optional(request),
        "gdc/gemini-2.5-pro",
    );
    assert_eq!(
        matches!(outcome, Err(GdcPolicyError::Parameter { .. })),
        rejected
    );
}

#[test]
fn a_non_string_project_fails_after_the_missing_location_check() {
    let request = optional(parsed(r#"{"vertex_project": 5}"#));
    let error = complete_url(
        Some("https://gateway"),
        &call(),
        &global(),
        &request,
        "gdc/gemini-2.5-pro",
    )
    .unwrap_err();
    assert!(matches!(
        &error,
        GdcPolicyError::Missing {
            what: "location",
            ..
        }
    ));
    assert_eq!(error_status(&error), 401);
}

#[rstest]
#[case::first_path("/v1/projects/pre/locations/eu", Some("pre"))]
#[case::path_after_an_empty_segment(
    "https://gateway/v1/projects//v1/projects/second/x",
    Some("second")
)]
#[case::terminal_capture("https://gateway/x/v1/projects/end", Some("end"))]
#[case::no_path("https://gateway", Some("project"))]
#[case::empty_capture_then_path("/v1/projects//x", Some("project"))]
#[case::truncated_prefix("/v1/projects/", Some("project"))]
fn effective_project_prefers_a_project_path_in_the_base(
    #[case] api_base: &str,
    #[case] expected: Option<&str>,
) {
    let request = optional(parsed(r#"{"vertex_project": "optional"}"#));
    let project = effective_project(api_base, &located_call(), &global(), &request);
    assert_eq!(project.as_ref().and_then(Json::as_str), expected);
}

#[rstest]
#[case::plain("abc", true)]
#[case::full_alphabet("a-Z_09", true)]
#[case::single_character("x", true)]
#[case::terminal_newline("abc\n", true)]
#[case::double_newline("abc\n\n", false)]
#[case::carriage_return("abc\r\n", false)]
#[case::empty("", false)]
#[case::only_newline("\n", false)]
#[case::space("a b", false)]
#[case::slash("a/b", false)]
#[case::dot("a.b", false)]
#[case::non_ascii("\u{e9}", false)]
#[case::tab("a\tb", false)]
fn validates_plain_identifiers_as_python_does(#[case] value: &str, #[case] accepted: bool) {
    let identifier = Json::Text(value.to_owned());
    let outcome = validate_path_id(&identifier, "vertex_project", "gdc/gemini-2.5-pro");
    assert_eq!(outcome.is_ok(), accepted);
    if accepted {
        assert_eq!(outcome.unwrap(), identifier.as_str().unwrap());
    }
}

#[test]
fn a_non_string_identifier_is_a_parameter_error() {
    let request = optional(parsed(r#"{"vertex_project": 5}"#));
    let value = request.get("vertex_project").unwrap();
    let error = validate_path_id(value, "vertex_project", "gdc/gemini-2.5-pro").unwrap_err();
    assert_eq!(error.class().metadata().status_code, 400);
}

#[rstest]
#[case::explicit_wins(
    Some("https://explicit"),
    Some("https://gdc"),
    Some("https://api"),
    "https://explicit/v1/projects/project/locations/location/chat/completions"
)]
#[case::empty_explicit_falls_through(
    Some(""),
    Some("https://gdc"),
    Some("https://api"),
    "https://gdc/v1/projects/project/locations/location/chat/completions"
)]
#[case::gdc_global_wins(
    None,
    Some("https://gdc"),
    Some("https://api"),
    "https://gdc/v1/projects/project/locations/location/chat/completions"
)]
#[case::api_global_is_last(
    None,
    Some(""),
    Some("https://api"),
    "https://api/v1/projects/project/locations/location/chat/completions"
)]
fn follows_the_base_order(
    #[case] explicit: Option<&str>,
    #[case] gdc_base: Option<&str>,
    #[case] api_base: Option<&str>,
    #[case] expected: &str,
) {
    let mut global = global();
    global.gdc_api_base = gdc_base.map(str::to_owned);
    global.api_base = api_base.map(str::to_owned);
    let url = complete_url(
        explicit,
        &located_call(),
        &global,
        &optional(parsed("{}")),
        "gdc/gemini-2.5-pro",
    )
    .unwrap();
    assert_eq!(url, expected);
}

#[test]
fn a_missing_base_is_authentication_failure() {
    let error = complete_url(
        None,
        &located_call(),
        &global(),
        &optional(parsed("{}")),
        "gdc/gemini-2.5-pro",
    )
    .unwrap_err();
    assert_eq!(error_status(&error), 401);
}

#[rstest]
#[case::plain_host(
    "gateway.example",
    "https://gateway.example/v1/projects/project/locations/location/chat/completions"
)]
#[case::already_https(
    "https://gateway.example",
    "https://gateway.example/v1/projects/project/locations/location/chat/completions"
)]
#[case::uppercase_scheme(
    "HTTP://X",
    "https://HTTP://X/v1/projects/project/locations/location/chat/completions"
)]
#[case::trailing_slashes(
    "https://gateway.example///",
    "https://gateway.example/v1/projects/project/locations/location/chat/completions"
)]
#[case::http_is_kept(
    "http://gateway.example",
    "http://gateway.example/v1/projects/project/locations/location/chat/completions"
)]
fn prefixes_https_unless_the_base_starts_with_http_and_trims_slashes(
    #[case] base: &str,
    #[case] expected: &str,
) {
    let url = complete_url(
        Some(base),
        &located_call(),
        &global(),
        &optional(parsed("{}")),
        "gdc/gemini-2.5-pro",
    )
    .unwrap();
    assert_eq!(url, expected);
}

#[rstest]
#[case::trailing_slash(
    "https://gateway/v1/projects/pre/locations/eu/",
    "https://gateway/v1/projects/pre/locations/eu"
)]
#[case::plain("https://gateway/v1/projects/pre", "https://gateway/v1/projects/pre")]
#[case::without_scheme("gateway/v1/projects/pre/", "https://gateway/v1/projects/pre")]
fn a_project_url_is_returned_unchanged_without_a_location(
    #[case] base: &str,
    #[case] expected: &str,
) {
    let url = complete_url(
        Some(base),
        &call(),
        &global(),
        &optional(parsed("{}")),
        "gdc/gemini-2.5-pro",
    )
    .unwrap();
    assert_eq!(url, expected);
}

#[test]
fn a_missing_project_is_authentication_failure() {
    let error = complete_url(
        Some("https://gateway"),
        &call(),
        &global(),
        &optional(parsed("{}")),
        "gdc/gemini-2.5-pro",
    )
    .unwrap_err();
    assert_eq!(error_status(&error), 401);
}

#[test]
fn a_missing_location_is_authentication_failure() {
    let mut call = call();
    call.vertex_project = Some("project".to_owned());
    let error = complete_url(
        Some("https://gateway"),
        &call,
        &global(),
        &optional(parsed("{}")),
        "gdc/gemini-2.5-pro",
    )
    .unwrap_err();
    assert_eq!(error_status(&error), 401);
}

#[rstest]
#[case::project("vertex_project", Some("bad/project"), Some("location"))]
#[case::location("vertex_location", Some("project"), Some("bad location"))]
fn an_invalid_identifier_is_authentication_failure(
    #[case] field: &str,
    #[case] project: Option<&str>,
    #[case] location: Option<&str>,
) {
    let mut call = call();
    call.vertex_project = project.map(str::to_owned);
    call.vertex_location = location.map(str::to_owned);
    let error = complete_url(
        Some("https://gateway"),
        &call,
        &global(),
        &optional(parsed("{}")),
        "gdc/gemini-2.5-pro",
    )
    .unwrap_err();
    assert_eq!(error_status(&error), 401);
    assert!(error.to_string().contains(field));
}

#[test]
fn a_terminal_newline_identifier_reaches_the_url() {
    let mut call = call();
    call.vertex_project = Some("project\n".to_owned());
    call.vertex_location = Some("location".to_owned());
    let url = complete_url(
        Some("https://gateway"),
        &call,
        &global(),
        &optional(parsed("{}")),
        "gdc/gemini-2.5-pro",
    )
    .unwrap();
    assert_eq!(
        url,
        "https://gateway/v1/projects/project\n/locations/location/chat/completions"
    );
}

#[rstest]
#[case::path(
    "https://gateway.example/v1/projects/p/locations/eu",
    "https://gateway.example"
)]
#[case::path_query_fragment(
    "https://gateway.example/some/path?q=1#frag",
    "https://gateway.example"
)]
#[case::no_scheme("gateway.example/path", "https://gateway.example")]
#[case::prefix_checked_before_parsing("HTTP://API.Example.COM:443/x", "https://HTTP:")]
#[case::userinfo_and_port("https://user:pw@host:8080/p?q=1#f", "https://user:pw@host:8080")]
#[case::scheme_lookalike("https://HTTPS://x", "https://HTTPS:")]
#[case::invalid_scheme_character("http_tp://x", "://")]
#[case::ipv6_host("https://[::1]:8080/x", "https://[::1]:8080")]
#[case::leading_space("  https://host/x", "https://  https:")]
#[case::control_characters("https://host\t/x", "https://host")]
#[case::query_only("https://host?q=1", "https://host")]
#[case::fragment_only("https://host#f", "https://host")]
#[case::empty_authority("https://", "https://")]
#[case::last_port("https://host:99999/x", "https://host:99999")]
#[case::ipv_future("https://[v1.foo]:443/x", "https://[v1.foo]:443")]
#[case::scoped_ipv6("https://[fe80::1%25eth0]:443/x", "https://[fe80::1%25eth0]:443")]
#[case::scoped_ipv6_without_encoding("https://[fe80::1%eth0]/x", "https://[fe80::1%eth0]")]
#[case::brackets_in_userinfo("https://u[]@v1.x", "https://u[]@v1.x")]
#[case::brackets_in_userinfo_with_port("https://u[x]@v1.x:443/p", "https://u[x]@v1.x:443")]
#[case::ipvfuture_in_userinfo("https://u[@v1.a]b", "https://u[@v1.a]b")]
#[case::nfkc_clean("https://gdc.ex\u{e9}mple", "https://gdc.ex\u{e9}mple")]
fn audience_is_the_scheme_and_netloc_of_the_original_base(
    #[case] base: &str,
    #[case] expected: &str,
) {
    assert_eq!(audience(base).unwrap(), expected);
}

#[rstest]
#[case::unclosed_bracket("https://[fd00::1/x")]
#[case::stray_closing_bracket("https://gdc]/x")]
#[case::data_before_the_bracket("https://a[::1]/x")]
#[case::bracketed_host_is_not_an_address("https://[notip]/x")]
#[case::suffix_after_the_bracket("https://[::1]x/path")]
#[case::empty_ipv_future_version("https://[v.foo]/x")]
#[case::non_hex_ipv_future_version("https://[vzz.foo]/x")]
#[case::empty_ipv_future_address("https://[v1.]/x")]
#[case::empty_scope_id("https://[fe80::1%]/x")]
#[case::scope_with_a_percent("https://[fe80::1%eth0%x]/x")]
#[case::brackets_in_userinfo_for_a_non_ip_host("https://u[]@example.com")]
#[case::empty_ipvfuture_hex_in_userinfo("https://u[]@v.a]b")]
#[case::empty_ipvfuture_suffix_in_userinfo("https://u[]@v1.")]
#[case::non_hex_ipvfuture_in_userinfo("https://u[]@vzz.x")]
#[case::nfkc_reintroduces_a_slash("https://gdc.example\u{2100}")]
#[case::nfkc_reintroduces_a_fullwidth_slash("https://gdc\u{ff0f}example")]
fn audience_rejects_what_urlsplit_rejects(#[case] base: &str) {
    let error = audience(base).unwrap_err();
    assert!(matches!(&error, GdcPolicyError::InvalidBase));
    assert_eq!(error.class().metadata().status_code, 500);
}

#[rstest]
#[case::explicit_bool_wins(
    Some(BoolOrString::Bool(false)),
    Some("true"),
    true,
    BoolOrString::Bool(false)
)]
#[case::explicit_false_word(Some(BoolOrString::Text("no".to_owned())), Some("true"), true, BoolOrString::Bool(false))]
#[case::explicit_true_word(Some(BoolOrString::Text(" TRUE ".to_owned())), None, true, BoolOrString::Bool(true))]
#[case::explicit_path(Some(BoolOrString::Text("ca.pem".to_owned())), None, true, BoolOrString::Text("ca.pem".to_owned()))]
#[case::explicit_path_keeps_spacing(Some(BoolOrString::Text(" ca.pem ".to_owned())), None, true, BoolOrString::Text(" ca.pem ".to_owned()))]
#[case::explicit_empty_is_used(
    Some(BoolOrString::Text(String::new())),
    Some("true"),
    true,
    BoolOrString::Text(String::new())
)]
#[case::environment_ignored_behind_a_bool(
    Some(BoolOrString::Bool(true)),
    Some("off"),
    true,
    BoolOrString::Bool(true)
)]
#[case::environment_path(None, Some("ca.pem"), true, BoolOrString::Text("ca.pem".to_owned()))]
#[case::empty_environment_is_the_default(None, Some(""), true, BoolOrString::Bool(true))]
#[case::absent_uses_the_true_default(None, None, true, BoolOrString::Bool(true))]
#[case::absent_uses_the_false_default(None, None, false, BoolOrString::Bool(false))]
fn the_argument_wins_and_unrecognized_text_keeps_its_original_form(
    #[case] value: Option<BoolOrString>,
    #[case] environment: Option<&str>,
    #[case] default: bool,
    #[case] expected: BoolOrString,
) {
    assert_eq!(
        read_env_bool("SSL_VERIFY", value.as_ref(), environment, default),
        expected
    );
}

#[rstest]
#[case::true_flag(BoolOrString::Bool(true), true)]
#[case::false_flag(BoolOrString::Bool(false), false)]
#[case::ca_path(BoolOrString::Text("ca.pem".to_owned()), true)]
#[case::empty_text(BoolOrString::Text(String::new()), false)]
#[case::whitespace_text(BoolOrString::Text(" ".to_owned()), true)]
fn token_caching_reads_a_nonempty_string_as_true(
    #[case] setting: BoolOrString,
    #[case] expected: bool,
) {
    assert_eq!(caching_enabled(&setting), expected);
}

fn transform(model: &str, messages_json: Value, optional_json: Value) -> Value {
    let typed_messages: Vec<ChatMessage> = serde_json::from_value(messages_json).unwrap();
    let params = optional(optional_json);
    let environment = BTreeMap::<String, String>::new();
    let body = transform_request(
        model,
        &typed_messages,
        &params,
        &environment,
        &RefuseDownloads,
    )
    .unwrap();
    serde_json::to_value(body).unwrap()
}

#[test]
fn strips_one_gdc_prefix_and_the_routing_parameters() {
    let body = transform(
        "gdc/gemini-2.5-pro",
        parsed(r#"[{"role": "user", "content": "hi"}]"#),
        parsed(
            r#"{"vertex_project": "p", "vertex_ai_project": "pa", "vertex_location": "l",
                "vertex_ai_location": "la", "ssl_verify": false, "gdc_token_caching": true,
                "temperature": 1}"#,
        ),
    );
    assert_eq!(
        body,
        parsed(
            r#"{"model": "gemini-2.5-pro", "messages": [{"role": "user", "content": "hi"}], "temperature": 1}"#
        )
    );
}

#[test]
fn strips_only_one_gdc_prefix() {
    let body = transform(
        "gdc/gdc/model",
        parsed(r#"[{"role": "user", "content": "hi"}]"#),
        parsed("{}"),
    );
    assert_eq!(body["model"], "gdc/model");
}

#[test]
fn keeps_a_model_without_the_provider_prefix() {
    let body = transform(
        "gemini",
        parsed(r#"[{"role": "user", "content": "hi"}]"#),
        parsed("{}"),
    );
    assert_eq!(body["model"], "gemini");
}

#[test]
fn the_parent_transform_drops_retries_and_an_unused_tool_choice() {
    let body = transform(
        "gdc/model",
        parsed(r#"[{"role": "user", "content": "hi"}]"#),
        parsed(r#"{"max_retries": 2, "tool_choice": "auto"}"#),
    );
    assert_eq!(
        body,
        parsed(r#"{"model": "model", "messages": [{"role": "user", "content": "hi"}]}"#)
    );
}

#[test]
fn the_parent_transform_strips_cache_control_from_content_parts() {
    let body = transform(
        "gdc/model",
        parsed(
            r#"[{"role": "user", "content": [{"type": "text", "text": "hi", "cache_control": {"type": "ephemeral"}}]}]"#,
        ),
        parsed("{}"),
    );
    assert_eq!(
        body,
        parsed(
            r#"{"model": "model", "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}]}]}"#
        )
    );
}

#[test]
fn authentication_failures_carry_the_401_class() {
    let error = GdcPolicyError::Missing {
        what: "project",
        model: "gdc/gemini-2.5-pro".to_owned(),
    };
    let metadata = error.class().metadata();
    assert_eq!(metadata.status_code, 401);
    assert_eq!(metadata.provider.as_deref(), Some("gdc"));
    assert_eq!(metadata.model.as_deref(), Some("gdc/gemini-2.5-pro"));
    assert_eq!(metadata.code, "401");
}

#[test]
fn a_request_parameter_of_the_wrong_type_is_a_400() {
    let error = GdcPolicyError::Parameter {
        param: "vertex_project",
    };
    assert_eq!(error.class().metadata().status_code, 400);
    assert!(error.to_string().contains("vertex_project"));
}
