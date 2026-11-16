use std::collections::BTreeMap;
use std::ptr;
use std::time::Duration;

use betterlite_config::sections::guardrail_identity::enums::SupportedGuardrailIntegrations;
use betterlite_config::sections::guardrail_params::mode::LitellmParamsMode;
use betterlite_config::sections::guardrail_params::params::LitellmParams;
use betterlite_http::proxy::base_builder;
use rstest::rstest;
use secrecy::ExposeSecret;
use tokio_util::task::TaskTracker;

use super::support::{environment, litellm_params};
use crate::noma_lifecycle::error::NomaLifecycleError;
use crate::noma_lifecycle::initializer::{
    BuildContext, GUARDRAIL_CLASS_REGISTRY, GUARDRAIL_INITIALIZER_REGISTRY, GuardrailRequest,
    InitContext, NomaRegistry, build_v2_guardrail, initialize_guardrail, initialize_guardrail_v2,
};
use crate::noma_lifecycle::{NomaCallback, NomaGuardrail};
use crate::noma_v2::guardrail::NomaV2Guardrail;

#[derive(Default)]
struct Recorder {
    callbacks: Vec<NomaCallback>,
}

impl NomaRegistry for Recorder {
    fn add_litellm_callback(&mut self, callback: NomaCallback) {
        self.callbacks.push(callback);
    }
}

const LEGACY_PARAMS: &str = r#"{"guardrail": "noma", "mode": "pre_call", "api_key": "k", "api_base": "http://noma.test", "application_id": "app", "monitor_mode": true, "block_failures": false, "anonymize_input": true, "default_on": true, "timeout": 3.0}"#;
const V2_PARAMS: &str = r#"{"guardrail": "noma_v2", "mode": "post_call", "api_base": "http://noma.test", "gateway_name": "gw", "monitor_mode": true}"#;

fn legacy(callback: &NomaCallback) -> &NomaGuardrail {
    match callback {
        NomaCallback::Legacy(guardrail) => guardrail,
        NomaCallback::V2(_) => panic!("expected the legacy guardrail"),
    }
}

fn v2(callback: &NomaCallback) -> &NomaV2Guardrail {
    match callback {
        NomaCallback::V2(guardrail) => guardrail,
        NomaCallback::Legacy(_) => panic!("expected the v2 guardrail"),
    }
}

fn request<'request>(
    params: &'request LitellmParams,
    guardrail_name: &'request str,
) -> GuardrailRequest<'request> {
    GuardrailRequest {
        params,
        guardrail_name,
    }
}

fn build_context(env: &BTreeMap<String, String>) -> BuildContext<'_> {
    BuildContext {
        env,
        client_builder: base_builder(),
        tasks: TaskTracker::new(),
        strict_modes: true,
    }
}

#[test]
fn the_registries_are_keyed_by_the_supported_integration_values() {
    let initializer_keys = GUARDRAIL_INITIALIZER_REGISTRY.map(|(key, _initializer)| key);
    let class_keys = GUARDRAIL_CLASS_REGISTRY.map(|(key, _class)| key);
    let expected = [
        SupportedGuardrailIntegrations::Noma,
        SupportedGuardrailIntegrations::NomaV2,
    ];
    assert_eq!(initializer_keys, expected);
    assert_eq!(class_keys, expected);
    assert_eq!(serde_json::to_string(&expected[0]).unwrap(), r#""noma""#);
    assert_eq!(serde_json::to_string(&expected[1]).unwrap(), r#""noma_v2""#);
}

#[test]
fn the_initializer_registry_maps_noma_and_noma_v2_to_their_initializers() {
    let env = environment(&[]);
    let [(_, noma_initializer), (_, v2_initializer)] = GUARDRAIL_INITIALIZER_REGISTRY;
    let mut recorder = Recorder::default();
    let context = InitContext {
        build: build_context(&env),
        registry: &mut recorder,
    };
    let built =
        noma_initializer(request(&litellm_params(LEGACY_PARAMS), "legacy"), context).unwrap();
    assert!(matches!(built, NomaCallback::Legacy(_)));
    let v2_context = InitContext {
        build: build_context(&env),
        registry: &mut recorder,
    };
    // the v2 entry builds v2 even though the parameters do not ask for it
    let v2_built =
        v2_initializer(request(&litellm_params(V2_PARAMS), "second"), v2_context).unwrap();
    assert!(matches!(v2_built, NomaCallback::V2(_)));
    assert_eq!(recorder.callbacks.len(), 2);
}

#[test]
fn the_class_registry_maps_noma_and_noma_v2_to_their_classes() {
    let env = environment(&[]);
    let [(_, noma_class), (_, v2_class)] = GUARDRAIL_CLASS_REGISTRY;
    let legacy_built = noma_class(
        request(&litellm_params(LEGACY_PARAMS), "legacy"),
        build_context(&env),
    )
    .unwrap();
    assert!(matches!(legacy_built, NomaCallback::Legacy(_)));
    let v2_built = v2_class(
        request(&litellm_params(V2_PARAMS), "second"),
        build_context(&env),
    )
    .unwrap();
    assert!(matches!(v2_built, NomaCallback::V2(_)));
}

#[test]
fn initialize_guardrail_builds_registers_and_returns_the_legacy_guardrail() {
    let env = environment(&[]);
    let mut recorder = Recorder::default();
    let context = InitContext {
        build: build_context(&env),
        registry: &mut recorder,
    };
    let returned =
        initialize_guardrail(request(&litellm_params(LEGACY_PARAMS), "my-noma"), context).unwrap();
    assert_eq!(recorder.callbacks.len(), 1);
    let guardrail = legacy(&recorder.callbacks[0]);
    assert!(ptr::eq(guardrail, legacy(&returned)));
    let settings = &guardrail.policy.settings;
    assert_eq!(settings.guardrail_name.as_deref(), Some("my-noma"));
    assert_eq!(
        settings.api_key.as_ref().map(ExposeSecret::expose_secret),
        Some("k")
    );
    assert_eq!(settings.api_base.as_deref(), Some("http://noma.test"));
    assert_eq!(settings.application_id.as_deref(), Some("app"));
    assert!(settings.monitor_mode);
    assert!(settings.anonymize_input);
    assert_eq!(settings.timeout, Some(Duration::from_secs(3)));
    assert!(!guardrail.block_failures);
    assert_eq!(
        settings.event_hook,
        Some(LitellmParamsMode::One(String::from("pre_call")))
    );
    assert!(guardrail.base.settings.default_on);
    assert_eq!(
        guardrail.base.settings.guardrail_name.as_deref(),
        Some("my-noma")
    );
}

#[test]
fn unset_parameters_fall_back_to_the_environment_and_the_defaults() {
    let env = environment(&[("NOMA_APPLICATION_ID", "from-env")]);
    let mut recorder = Recorder::default();
    let context = InitContext {
        build: build_context(&env),
        registry: &mut recorder,
    };
    let configured = litellm_params(r#"{"guardrail": "noma", "mode": "pre_call"}"#);
    initialize_guardrail(request(&configured, "plain"), context).unwrap();
    let guardrail = legacy(&recorder.callbacks[0]);
    assert_eq!(
        guardrail.policy.settings.application_id.as_deref(),
        Some("from-env")
    );
    assert!(!guardrail.policy.settings.monitor_mode);
    assert!(guardrail.block_failures);
    assert!(!guardrail.base.settings.default_on);
}

#[rstest]
#[case::boolean_true(r#""use_v2": true"#, true)]
#[case::text_true(r#""use_v2": "true""#, true)]
#[case::text_upper_case(r#""use_v2": "TRUE""#, true)]
#[case::boolean_false(r#""use_v2": false"#, false)]
#[case::text_false(r#""use_v2": "false""#, false)]
#[case::text_yes(r#""use_v2": "yes""#, true)]
#[case::text_empty(r#""use_v2": """#, false)]
#[case::null(r#""use_v2": null"#, false)]
#[case::absent(r#""other": 1"#, false)]
fn use_v2_routes_the_initializer(#[case] entry: &str, #[case] routed_to_v2: bool) {
    let env = environment(&[]);
    let mut recorder = Recorder::default();
    let context = InitContext {
        build: build_context(&env),
        registry: &mut recorder,
    };
    let body = format!(
        r#"{{"guardrail": "noma", "mode": "pre_call", "api_base": "http://noma.test", {entry}}}"#
    );
    let returned =
        initialize_guardrail(request(&litellm_params(&body), "routed"), context).unwrap();
    assert_eq!(matches!(returned, NomaCallback::V2(_)), routed_to_v2);
    assert_eq!(recorder.callbacks.len(), 1);
    assert_eq!(
        matches!(recorder.callbacks[0], NomaCallback::V2(_)),
        routed_to_v2
    );
}

// the entries are configured through the initializer, which must reject them without registering anything
fn initialize_rejected(entry: &str) -> NomaLifecycleError {
    let env = environment(&[]);
    let mut recorder = Recorder::default();
    let context = InitContext {
        build: build_context(&env),
        registry: &mut recorder,
    };
    let body = format!(r#"{{"guardrail": "noma", "mode": "pre_call", {entry}}}"#);
    let error = initialize_guardrail(request(&litellm_params(&body), "bad"), context).unwrap_err();
    assert!(recorder.callbacks.is_empty());
    error
}

#[rstest]
#[case::number(r#""use_v2": 1"#)]
#[case::text_that_is_no_boolean(r#""use_v2": "maybe""#)]
fn a_use_v2_that_is_no_boolean_is_an_error(#[case] entry: &str) {
    let error = initialize_rejected(entry);
    assert!(matches!(
        error,
        NomaLifecycleError::Params(_) | NomaLifecycleError::Environment(_)
    ));
}

#[test]
fn initialize_guardrail_v2_builds_registers_and_returns_the_v2_guardrail() {
    let env = environment(&[]);
    let mut recorder = Recorder::default();
    let context = InitContext {
        build: build_context(&env),
        registry: &mut recorder,
    };
    let returned =
        initialize_guardrail_v2(request(&litellm_params(V2_PARAMS), "my-v2"), context).unwrap();
    assert_eq!(recorder.callbacks.len(), 1);
    let guardrail = v2(&recorder.callbacks[0]);
    assert!(ptr::eq(guardrail, v2(&returned)));
    assert_eq!(guardrail.settings.gateway_name.as_deref(), Some("gw"));
    assert!(guardrail.settings.monitor_mode);
    assert_eq!(
        guardrail.base.settings.guardrail_name.as_deref(),
        Some("my-v2")
    );
}

#[test]
fn the_v2_guardrail_reads_the_environment_it_is_given() {
    let env = environment(&[("NOMA_GATEWAY_NAME", "env-gateway")]);
    let configured = litellm_params(
        r#"{"guardrail": "noma_v2", "mode": "post_call", "api_base": "http://noma.test"}"#,
    );
    let built = build_v2_guardrail(request(&configured, "from-env"), build_context(&env)).unwrap();
    assert_eq!(
        v2(&built).settings.gateway_name.as_deref(),
        Some("env-gateway")
    );
}

#[rstest]
#[case::monitor_mode(r#""monitor_mode": "yes""#)]
#[case::block_failures(r#""block_failures": 1"#)]
#[case::anonymize_input(r#""anonymize_input": "no""#)]
#[case::application_id(r#""application_id": 7"#)]
fn a_parameter_of_the_wrong_type_is_an_error(#[case] entry: &str) {
    let error = initialize_rejected(entry);
    assert!(matches!(error, NomaLifecycleError::Params(_)));
}
