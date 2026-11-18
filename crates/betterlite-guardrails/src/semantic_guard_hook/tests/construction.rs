use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use rstest::rstest;

use crate::guardrail_base::error::GuardrailBaseError;
use crate::semantic_guard_hook::error::{NO_ROUTES_MESSAGE, SemanticGuardSetupError};
use crate::semantic_guard_hook::{
    DEFAULT_EVENT_HOOK, GUARDRAIL_PROVIDER, SUPPORTED_EVENT_HOOKS, SWITCHES,
};

use super::support::{
    FakeBuilder, args, calls, guardrail, guardrail_with_modes, route, routes_file,
    with_captured_logs,
};

#[test]
fn no_routes_are_refused_with_the_documented_message() {
    let mut given = args();
    given.custom_routes.clear();
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let Err(error) = guardrail_with_modes(given, &builder, true) else {
        panic!("the guardrail accepted an empty route list");
    };
    assert!(matches!(error, SemanticGuardSetupError::NoRoutes));
    assert_eq!(error.to_string(), NO_ROUTES_MESSAGE);
    assert!(builder.builder_calls.lock().unwrap().is_empty());
}

#[test]
fn the_routes_reach_the_builder_with_the_global_threshold() {
    let mut given = args();
    given.similarity_threshold = 0.6_f64;
    given.custom_routes.push(route("second", &["other text"]));
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let built = guardrail(given, &builder);
    assert_eq!(built.route_count, 2);
    let builder_calls = builder.builder_calls.lock().unwrap();
    assert_eq!(builder_calls.len(), 1);
    assert_eq!(
        builder_calls[0].names,
        vec!["attack".to_owned(), "second".to_owned()]
    );
    assert_eq!(builder_calls[0].threshold.to_bits(), 0.6_f64.to_bits());
}

#[test]
fn a_custom_routes_file_adds_its_routes() {
    let (_folder, path) = routes_file("- route_name: file_route\n  utterances: [\"file text\"]\n");
    let mut given = args();
    given.custom_routes_file = Some(path);
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let built = guardrail(given, &builder);
    assert_eq!(built.route_count, 2);
    let builder_calls = builder.builder_calls.lock().unwrap();
    assert_eq!(
        builder_calls[0].names,
        vec!["file_route".to_owned(), "attack".to_owned()]
    );
}

#[test]
fn a_route_template_adds_its_routes() {
    let mut given = args();
    given.route_templates = vec!["sql_injection".to_owned()];
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let built = guardrail(given, &builder);
    assert_eq!(built.route_count, 2);
    let builder_calls = builder.builder_calls.lock().unwrap();
    assert_eq!(
        builder_calls[0].names,
        vec!["sql_injection".to_owned(), "attack".to_owned()]
    );
}

#[test]
fn a_build_failure_is_reported() {
    let builder = FakeBuilder::build_failing(calls());
    let Err(error) = guardrail_with_modes(args(), &builder, true) else {
        panic!("the guardrail accepted a failed router build");
    };
    assert!(matches!(error, SemanticGuardSetupError::Build(_)));
    assert_eq!(
        error.to_string(),
        "the semantic router could not be built: the fake semantic router build failed"
    );
}

#[test]
fn the_constructed_guardrail_logs_its_settings() {
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let (built, output) = with_captured_logs(|| guardrail(args(), &builder));
    assert_eq!(built.route_count, 1);
    assert!(output.contains(
        "SemanticGuardrail 'semantic-guard' initialized with 1 routes, \
         embedding_model=text-embedding-3-small, threshold=0.75"
    ));
}

#[test]
fn the_provider_is_semantic_guard() {
    assert_eq!(GUARDRAIL_PROVIDER, "semantic_guard");
}

#[test]
fn the_class_settings_carry_the_guardrail_configuration() {
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let built = guardrail(args(), &builder);
    let settings = built.base.settings;
    assert_eq!(settings.guardrail_name.as_deref(), Some("semantic-guard"));
    assert_eq!(
        settings.supported_event_hooks.as_deref(),
        Some(SUPPORTED_EVENT_HOOKS.as_slice())
    );
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One("pre_call".to_owned()))
    );
    assert_eq!(settings.switches, SWITCHES);
    assert!(!settings.default_on);
    assert_eq!(built.embedding_model, "text-embedding-3-small");
    assert_eq!(built.similarity_threshold.to_bits(), 0.75_f64.to_bits());
    assert_eq!(built.on_flagged_action, "block");
}

#[rstest]
#[case::absent(None)]
#[case::empty_one(Some(LitellmParamsMode::One(String::new())))]
#[case::empty_many(Some(LitellmParamsMode::Many(Vec::new())))]
fn an_unset_event_hook_defaults_to_pre_call(#[case] event_hook: Option<LitellmParamsMode>) {
    let mut given = args();
    given.event_hook = event_hook;
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let built = guardrail(given, &builder);
    assert_eq!(
        built.base.settings.event_hook,
        Some(LitellmParamsMode::One(DEFAULT_EVENT_HOOK.to_owned()))
    );
}

#[test]
fn a_configured_event_hook_is_kept() {
    let mut given = args();
    given.event_hook = Some(LitellmParamsMode::One("post_call".to_owned()));
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let built = guardrail(given, &builder);
    assert_eq!(
        built.base.settings.event_hook,
        Some(LitellmParamsMode::One("post_call".to_owned()))
    );
}

#[test]
fn a_configured_many_event_hook_is_kept() {
    let mut given = args();
    given.event_hook = Some(LitellmParamsMode::Many(vec!["post_call".to_owned()]));
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let built = guardrail(given, &builder);
    assert_eq!(
        built.base.settings.event_hook,
        Some(LitellmParamsMode::Many(vec!["post_call".to_owned()]))
    );
}

#[test]
fn default_on_is_copied_to_the_class_settings() {
    let mut given = args();
    given.default_on = true;
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let built = guardrail(given, &builder);
    assert!(built.base.settings.default_on);
}

#[test]
fn an_unsupported_event_hook_is_refused_in_strict_mode() {
    let mut given = args();
    given.event_hook = Some(LitellmParamsMode::One("during_call".to_owned()));
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let Err(error) = guardrail_with_modes(given, &builder, true) else {
        panic!("the guardrail accepted an unsupported event hook");
    };
    assert!(matches!(
        error,
        SemanticGuardSetupError::Base(GuardrailBaseError::UnsupportedEventHook { .. })
    ));
}

#[test]
fn an_unsupported_event_hook_warns_when_not_strict() {
    let mut given = args();
    given.event_hook = Some(LitellmParamsMode::One("during_call".to_owned()));
    let builder = FakeBuilder::matching(Vec::new(), calls());
    let (outcome, output) = with_captured_logs(|| guardrail_with_modes(given, &builder, false));
    let built = outcome.unwrap();
    assert_eq!(
        built.base.settings.event_hook,
        Some(LitellmParamsMode::One("during_call".to_owned()))
    );
    assert!(output.contains("LITELLM_STRICT_GUARDRAIL_MODES=false"));
}
