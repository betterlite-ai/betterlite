use betterlite_models::controls::call_params::CallParams;
use betterlite_models::controls::optional_params::OptionalParams;
use betterlite_models::headers::affinity::AffinityParams;
use betterlite_models::resolution::ResolutionError;
use rstest::rstest;
use secrecy::{ExposeSecret as _, SecretString};

use super::{capable_native, configs, dynamic_resolution, json, parameters, resolution, resolver};
use crate::api::responses_provider_selection::ParameterMap;
use crate::api::responses_provider_selection::bridging::{NEVER_BRIDGED, bridge_kwargs};
use crate::api::responses_provider_selection::prefixes::{
    normalize_openai_chat_model, pop_use_chat_completions_api, strip_responses_routing_prefix,
};
use crate::api::responses_provider_selection::prompting::{
    ProvisionalBridge, api_base_kwarg, will_use_chat_bridge,
};
use crate::api::responses_provider_selection::provider_config::ResponsesApiConfig;
use crate::api::responses_provider_selection::resolution::resolve_model_provider;

fn empty_params() -> CallParams {
    CallParams {
        model: None,
        api_key: None,
        api_base: None,
        custom_llm_provider: None,
        metadata: None,
        litellm_metadata: None,
        litellm_session_id: None,
        litellm_trace_id: None,
        no_log: None,
        drop_params: None,
        control: None,
        credentials: None,
        optional: OptionalParams::default(),
        affinity: AffinityParams::default(),
    }
}

#[rstest]
#[case::plain("gpt-4o", "gpt-4o", false)]
#[case::prefixed("openai/chat_completions/gpt-4o", "openai/gpt-4o", true)]
#[case::empty_remainder("openai/chat_completions/", "openai/chat_completions/", false)]
#[case::already_openai("openai/gpt-4o", "openai/gpt-4o", false)]
#[case::case_sensitive(
    "OpenAI/chat_completions/gpt-4o",
    "OpenAI/chat_completions/gpt-4o",
    false
)]
fn the_bridge_prefix_is_normalized(
    #[case] model: &str,
    #[case] expected: &str,
    #[case] prefixed: bool,
) {
    assert_eq!(
        normalize_openai_chat_model(model),
        (expected.to_owned(), prefixed)
    );
}

#[rstest]
#[case::stripped("responses/gpt-4o", "gpt-4o")]
#[case::one_segment("responses/responses/gpt-4o", "responses/gpt-4o")]
#[case::only_segment("responses/", "")]
#[case::plain("gpt-4o", "gpt-4o")]
#[case::case_sensitive("Responses/gpt-4o", "Responses/gpt-4o")]
fn the_routing_prefix_is_stripped_once(#[case] model: &str, #[case] expected: &str) {
    assert_eq!(strip_responses_routing_prefix(model), expected);
}

#[rstest]
#[case::absent(None, false)]
#[case::false_value(Some("false"), false)]
#[case::zero(Some("0"), false)]
#[case::zero_float(Some("0.0"), false)]
#[case::empty_text(Some(r#""""#), false)]
#[case::null(Some("null"), false)]
#[case::empty_list(Some("[]"), false)]
#[case::empty_object(Some("{}"), false)]
#[case::true_value(Some("true"), true)]
#[case::false_text(Some(r#""false""#), true)]
#[case::positive_number(Some("2"), true)]
#[case::one_element_list(Some("[0]"), true)]
fn the_bridge_flag_is_popped_with_python_truthiness(
    #[case] raw: Option<&str>,
    #[case] expected: bool,
) {
    let mut kwargs = ParameterMap::new();
    if let Some(text) = raw {
        kwargs.insert(String::from("use_chat_completions_api"), json(text));
    }
    assert_eq!(pop_use_chat_completions_api(&mut kwargs), expected);
    assert!(!kwargs.contains_key("use_chat_completions_api"));
}

#[test]
fn without_a_config_only_responses_only_fields_are_dropped() {
    let kwargs = parameters(&[
        ("client_metadata", json("{}")),
        ("temperature", json("0.2")),
        ("vendor_option", json(r#""x""#)),
        ("api_key", json(r#""sk-test""#)),
    ]);
    let kept = bridge_kwargs(&kwargs, None, &[]);
    assert!(!kept.contains_key(NEVER_BRIDGED));
    assert_eq!(
        kept.keys().cloned().collect::<Vec<_>>(),
        vec!["temperature", "vendor_option", "api_key"]
    );
}

#[test]
fn a_resolved_config_keeps_only_forwarded_parameter_names() {
    let kwargs = parameters(&[
        ("temperature", json("0.2")),
        ("api_key", json(r#""sk-test""#)),
        ("vendor_option", json(r#""x""#)),
        ("client_metadata", json("{}")),
        ("custom_option", json("1")),
    ]);
    let allowed = vec![String::from("custom_option")];
    let kept = bridge_kwargs(&kwargs, Some(&ResponsesApiConfig::OpenAiLike), &allowed);
    assert_eq!(
        kept.keys().cloned().collect::<Vec<_>>(),
        vec!["temperature", "api_key", "custom_option"]
    );
}

#[test]
fn a_request_without_a_provider_always_bridges() {
    let lookup = configs(&[]);
    let model_info = json("{}");
    assert!(will_use_chat_bridge(
        &lookup,
        &ProvisionalBridge {
            model: "gpt-4o",
            custom_llm_provider: None,
            use_chat_completions_api: false,
            model_info: &model_info,
            api_base: None,
        }
    ));
}

#[test]
fn an_unknown_provider_bridges_without_a_config() {
    let lookup = configs(&[]);
    let model_info = json("{}");
    assert!(will_use_chat_bridge(
        &lookup,
        &ProvisionalBridge {
            model: "m",
            custom_llm_provider: Some("mystery"),
            use_chat_completions_api: false,
            model_info: &model_info,
            api_base: None,
        }
    ));
}

#[test]
fn a_native_deployment_bridges_only_for_a_prefixed_model_or_an_explicit_request() {
    let lookup = configs(&[("openai", capable_native())]);
    let model_info = json("{}");
    let native = ProvisionalBridge {
        model: "gpt-4o",
        custom_llm_provider: Some("openai"),
        use_chat_completions_api: false,
        model_info: &model_info,
        api_base: None,
    };
    assert!(!will_use_chat_bridge(&lookup, &native));
    let prefixed = ProvisionalBridge {
        model: "openai/chat_completions/gpt-4o",
        custom_llm_provider: Some("openai"),
        use_chat_completions_api: false,
        model_info: &model_info,
        api_base: None,
    };
    assert!(will_use_chat_bridge(&lookup, &prefixed));
}

#[test]
fn an_explicit_chat_completions_request_bridges_a_native_deployment() {
    let lookup = configs(&[("openai", capable_native())]);
    let model_info = json("{}");
    assert!(will_use_chat_bridge(
        &lookup,
        &ProvisionalBridge {
            model: "gpt-4o",
            custom_llm_provider: Some("openai"),
            use_chat_completions_api: true,
            model_info: &model_info,
            api_base: None,
        }
    ));
}

#[test]
fn the_compatible_config_is_not_a_bridge() {
    let lookup = configs(&[]);
    let model_info = json(r#"{"supported_endpoints":["/v1/responses"]}"#);
    assert!(!will_use_chat_bridge(
        &lookup,
        &ProvisionalBridge {
            model: "gpt-4o",
            custom_llm_provider: Some("openai_like"),
            use_chat_completions_api: false,
            model_info: &model_info,
            api_base: None,
        }
    ));
}

#[rstest]
#[case::absent(&[], None)]
#[case::text(&[("api_base", r#""https://x.test/v1""#)], Some("https://x.test/v1"))]
#[case::null(&[("api_base", "null")], None)]
#[case::number(&[("api_base", "7")], None)]
fn only_a_text_api_base_reaches_the_lookup(
    #[case] pairs: &[(&str, &str)],
    #[case] expected: Option<&str>,
) {
    let kwargs = pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), json(value)))
        .collect::<ParameterMap>();
    assert_eq!(api_base_kwarg(&kwargs), expected);
}

#[test]
fn an_explicit_provider_fills_an_empty_parameter() {
    let resolver = resolver(&[("gpt-4o", resolution("gpt-4o", "azure"))]);
    let mut params = CallParams::default();
    let mut local_vars = ParameterMap::new();
    let outcome = resolve_model_provider(
        &resolver,
        "gpt-4o",
        Some("azure"),
        &mut params,
        &mut local_vars,
    )
    .expect("resolution succeeds");
    assert_eq!(outcome, (String::from("gpt-4o"), String::from("azure")));
    assert_eq!(params.custom_llm_provider.as_deref(), Some("azure"));
    assert_eq!(
        local_vars.get("custom_llm_provider"),
        Some(&json(r#""azure""#))
    );
    let calls = resolver.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].custom_llm_provider.as_deref(), Some("azure"));
}

#[test]
fn an_existing_parameter_beats_the_request_provider() {
    let resolver = resolver(&[("gpt-4o", resolution("gpt-4o", "openai"))]);
    let mut params = empty_params();
    params.custom_llm_provider = Some(String::from("openai"));
    let mut local_vars = ParameterMap::new();
    resolve_model_provider(
        &resolver,
        "gpt-4o",
        Some("azure"),
        &mut params,
        &mut local_vars,
    )
    .expect("resolution succeeds");
    assert_eq!(params.custom_llm_provider.as_deref(), Some("openai"));
    let calls = resolver.calls();
    assert_eq!(calls[0].custom_llm_provider.as_deref(), Some("openai"));
}

#[test]
fn an_empty_parameter_is_filled_by_the_request_provider() {
    let resolver = resolver(&[("gpt-4o", resolution("gpt-4o", "azure"))]);
    let mut params = empty_params();
    params.custom_llm_provider = Some(String::new());
    let mut local_vars = ParameterMap::new();
    resolve_model_provider(
        &resolver,
        "gpt-4o",
        Some("azure"),
        &mut params,
        &mut local_vars,
    )
    .expect("resolution succeeds");
    assert_eq!(params.custom_llm_provider.as_deref(), Some("azure"));
}

#[test]
fn a_dynamic_credential_replaces_the_callers() {
    let resolver = resolver(&[(
        "gpt-4o",
        dynamic_resolution(
            "responses/gpt-4o",
            "openai",
            Some("sk-dynamic"),
            Some("https://dynamic.test/v1"),
        ),
    )]);
    let mut params = empty_params();
    params.api_key = Some(SecretString::from("sk-caller"));
    params.api_base = Some(String::from("https://caller.test/v1"));
    let mut local_vars = ParameterMap::new();
    let outcome = resolve_model_provider(&resolver, "gpt-4o", None, &mut params, &mut local_vars)
        .expect("resolution succeeds");
    assert_eq!(outcome, (String::from("gpt-4o"), String::from("openai")));
    assert_eq!(
        params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-dynamic")
    );
    assert_eq!(params.api_base.as_deref(), Some("https://dynamic.test/v1"));
    let calls = resolver.calls();
    assert_eq!(calls[0].api_key.as_deref(), Some("sk-caller"));
}

#[test]
fn an_unresolved_credential_keeps_the_callers() {
    let resolver = resolver(&[("gpt-4o", resolution("responses/responses/gpt-4o", "openai"))]);
    let mut params = empty_params();
    params.api_key = Some(SecretString::from("sk-caller"));
    params.api_base = Some(String::from("https://caller.test/v1"));
    let mut local_vars = ParameterMap::new();
    let outcome = resolve_model_provider(&resolver, "gpt-4o", None, &mut params, &mut local_vars)
        .expect("resolution succeeds");
    assert_eq!(
        outcome,
        (String::from("responses/gpt-4o"), String::from("openai"))
    );
    assert_eq!(
        params
            .api_key
            .as_ref()
            .map(|key| key.expose_secret().as_str()),
        Some("sk-caller")
    );
    assert_eq!(params.api_base.as_deref(), Some("https://caller.test/v1"));
}

#[test]
fn an_unresolvable_model_is_an_error() {
    let resolver = resolver(&[]);
    let mut params = CallParams::default();
    let mut local_vars = ParameterMap::new();
    let error = resolve_model_provider(
        &resolver,
        "unknown/model",
        None,
        &mut params,
        &mut local_vars,
    )
    .expect_err("an unregistered model names no provider");
    assert!(matches!(error, ResolutionError::MissingProvider { .. }));
}
