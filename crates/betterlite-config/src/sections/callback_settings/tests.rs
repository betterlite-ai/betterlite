#![cfg(test)]

use std::collections::BTreeMap;

use indexmap::IndexMap;
use rstest::rstest;

use super::names::CallbackName;
use super::{RawCallbackList, Settings};
use crate::loader::{ConfigError, ConfigLoader};

const EXPECTED_NAMES: [&str; 73] = [
    "lago",
    "openmeter",
    "logfire",
    "literalai",
    "litellm_agent",
    "dynamic_rate_limiter",
    "dynamic_rate_limiter_v3",
    "langsmith",
    "prometheus",
    "otel",
    "datadog",
    "datadog_metrics",
    "datadog_llm_observability",
    "galileo",
    "braintrust",
    "arize",
    "arize_phoenix",
    "langtrace",
    "gcs_bucket",
    "azure_storage",
    "opik",
    "argilla",
    "mlflow",
    "langfuse",
    "langfuse_otel",
    "weave_otel",
    "pagerduty",
    "humanloop",
    "azure_sentinel",
    "gcs_pubsub",
    "agentops",
    "anthropic_cache_control_hook",
    "generic_api",
    "resend_email",
    "sendgrid_email",
    "smtp_email",
    "deepeval",
    "s3_v2",
    "clickhouse",
    "pointfive",
    "zerobus",
    "aws_sqs",
    "vector_store_pre_call_hook",
    "dotprompt",
    "bitbucket",
    "gitlab",
    "cloudzero",
    "focus",
    "mavvrik",
    "vantage",
    "posthog",
    "levo",
    "compression_interception",
    "newrelic",
    "signoz",
    "sentry",
    "slack",
    "traceloop",
    "athina",
    "helicone",
    "lunary",
    "promptlayer",
    "dynamodb",
    "s3",
    "wandb",
    "supabase",
    "greenscale",
    "datadog_cost_management",
    "sample_callback",
    "rubrik",
    "sumologic",
    "qualifire_eval",
    "ramp",
];

enum Scenario {
    EmptyLists,
    AcceptedName(&'static str, &'static str),
    RejectedName(&'static str, &'static str),
    EveryList(&'static [&'static str]),
    NameOrder,
    NameRoundTrip,
    SettingsScalars,
    SettingsDebugRedacts,
    Capacity(&'static [(&'static str, &'static str)], usize),
    StringCallbacks(&'static str),
    ScalarSuccessCallback(&'static str),
    ScalarEntry(&'static str),
    ResolvedSetting(&'static [(&'static str, &'static str)]),
    MissingSetting,
    NestedOwner,
    OtelUnknownScalar,
    OtelUnknownNested,
    NestedOwnerless,
    CallbackListShapes,
}

fn assert_every_name_round_trips() {
    for callback in CallbackName::ALL.iter().copied() {
        assert_eq!(CallbackName::from_name(callback.as_str()), Some(callback));
    }
}

fn load(source: &str, environment: &[(&str, &str)]) -> Result<Settings, ConfigError> {
    let lookup: BTreeMap<String, String> = environment
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let config = ConfigLoader { lookup }.load(source)?;
    Ok(config.sections.callback_settings)
}

fn variables(pairs: &[(&str, &str)]) -> IndexMap<String, String> {
    pairs
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[rstest]
#[case::empty_lists_load(Scenario::EmptyLists)]
#[case::langsmith_in_callbacks_loads(Scenario::AcceptedName("callbacks", "langsmith"))]
#[case::s3_in_success_callback_loads(Scenario::AcceptedName("success_callback", "s3"))]
#[case::rubrik_in_failure_callback_loads(Scenario::AcceptedName("failure_callback", "rubrik"))]
#[case::not_a_callback_fails_with_its_list_path(Scenario::RejectedName(
    "callbacks",
    "not_a_callback"
))]
#[case::hide_secrets_fails_with_its_list_path(Scenario::RejectedName(
    "success_callback",
    "hide_secrets"
))]
#[case::every_unknown_list_is_named(Scenario::EveryList(&[
    "litellm_settings.callbacks",
    "litellm_settings.success_callback",
    "litellm_settings.failure_callback",
]))]
#[case::all_names_are_listed_in_order(Scenario::NameOrder)]
#[case::every_name_round_trips(Scenario::NameRoundTrip)]
#[case::callback_settings_scalars_keep_their_text(Scenario::SettingsScalars)]
#[case::callback_settings_debug_redacts_credentials(Scenario::SettingsDebugRedacts)]
#[case::capacity_defaults_to_one_hundred(Scenario::Capacity(&[], 100))]
#[case::capacity_reads_the_environment(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "7")], 7))]
#[case::empty_capacity_environment_takes_the_default(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "")], 100))]
#[case::invalid_capacity_environment_takes_the_default(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "many")], 100))]
#[case::negative_capacity_admits_nothing(Scenario::Capacity(&[("LITELLM_MAX_CALLBACKS", "-1")], 0))]
#[case::single_string_callbacks_fail_startup(Scenario::StringCallbacks(
    "litellm_settings:\n  callbacks: custom_callbacks.proxy_handler_instance\n"
))]
#[case::scalar_success_callback_fails_as_yaml(Scenario::ScalarSuccessCallback(
    "litellm_settings:\n  success_callback: langfuse\n"
))]
#[case::non_string_callback_entry_fails_as_yaml(Scenario::ScalarEntry(
    "litellm_settings:\n  callbacks: [1]\n"
))]
#[case::callback_setting_resolves_an_environment_reference(Scenario::ResolvedSetting(&[
    ("CALLBACK_SECRET", "sk-resolved"),
]))]
#[case::callback_setting_missing_environment_variable_fails(Scenario::MissingSetting)]
#[case::nested_settings_value_owned_by_another_section(Scenario::NestedOwner)]
#[case::unknown_otel_scalar_is_kept(Scenario::OtelUnknownScalar)]
#[case::unknown_otel_nested_value_is_unsupported(Scenario::OtelUnknownNested)]
#[case::nested_settings_value_without_an_owner_is_unsupported(Scenario::NestedOwnerless)]
#[case::callback_list_shapes_keep_their_names(Scenario::CallbackListShapes)]
fn callback_registry_tests_behavior(#[case] scenario: Scenario) {
    match scenario {
        Scenario::EmptyLists => {
            let settings = load(
                "litellm_settings:\n  callbacks: []\n  success_callback: []\n  failure_callback: []\n",
                &[],
            )
            .unwrap();
            assert!(settings.callbacks.is_empty());
            assert!(settings.success_callback.is_empty());
            assert!(settings.failure_callback.is_empty());
            assert!(settings.callback_settings.is_empty());
            assert_eq!(settings.max_callbacks, 100);
        }
        Scenario::AcceptedName(list, name) => {
            let source = format!("litellm_settings:\n  {list}: [\"{name}\"]\n");
            let settings = load(&source, &[]).unwrap();
            let stored = match list {
                "callbacks" => &settings.callbacks,
                "success_callback" => &settings.success_callback,
                _ => &settings.failure_callback,
            };
            assert_eq!(stored, &[name.to_owned()]);
        }
        Scenario::RejectedName(list, name) => {
            let source = format!("litellm_settings:\n  {list}: [\"{name}\"]\n");
            let error = load(&source, &[]).unwrap_err();
            let path = format!("litellm_settings.{list}");
            assert!(matches!(error, ConfigError::Unsupported { paths } if paths == [path]));
        }
        Scenario::EveryList(paths) => {
            let error = load(
                "litellm_settings:\n  callbacks: [not_a_callback]\n  success_callback: [not_a_callback]\n  failure_callback: [not_a_callback]\n",
                &[],
            )
            .unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths: found } if found == paths));
        }
        Scenario::NameOrder => {
            assert_eq!(CallbackName::ALL.len(), 73);
            let listed: Vec<&str> = CallbackName::ALL
                .iter()
                .map(|callback| callback.as_str())
                .collect();
            assert_eq!(listed, EXPECTED_NAMES);
        }
        Scenario::NameRoundTrip => assert_every_name_round_trips(),
        Scenario::SettingsScalars => {
            let settings = load(
                "callback_settings:\n  langfuse:\n    endpoint: https://collector\n    capture: true\n    retries: 3\n    timeout: 1.5\n    optional: null\n",
                &[],
            )
            .unwrap();
            let entry = settings
                .callback_settings
                .get("langfuse")
                .expect("langfuse settings are kept");
            assert_eq!(
                entry.exposed_variables(),
                variables(&[
                    ("endpoint", "https://collector"),
                    ("capture", "True"),
                    ("retries", "3"),
                    ("timeout", "1.5"),
                ])
            );
            let debug = format!("{settings:?}");
            assert!(debug.contains("optional"));
            assert!(debug.contains("Null"));
        }
        Scenario::SettingsDebugRedacts => {
            let settings = load(
                "callback_settings:\n  langfuse:\n    langfuse_secret: sk-secret\n    headers: api-key=secret\n",
                &[],
            )
            .unwrap();
            let debug = format!("{settings:?}");
            assert!(!debug.contains("sk-secret"));
            assert!(!debug.contains("api-key=secret"));
            let entry = settings
                .callback_settings
                .get("langfuse")
                .expect("langfuse settings are kept");
            assert_eq!(
                entry.exposed_variables(),
                variables(&[
                    ("langfuse_secret", "sk-secret"),
                    ("headers", "api-key=secret"),
                ])
            );
        }
        Scenario::Capacity(environment, expected) => {
            let settings = load("", environment).unwrap();
            assert_eq!(settings.max_callbacks, expected);
        }
        Scenario::StringCallbacks(source) => {
            let error = load(source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths }
                if paths == ["litellm_settings.callbacks"]));
        }
        Scenario::ScalarSuccessCallback(source) => {
            let error = load(source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Yaml { .. }));
        }
        Scenario::ResolvedSetting(environment) => {
            let settings = load(
                "callback_settings:\n  langfuse:\n    langfuse_secret: os.environ/CALLBACK_SECRET\n",
                environment,
            )
            .unwrap();
            let entry = settings
                .callback_settings
                .get("langfuse")
                .expect("langfuse settings are kept");
            assert_eq!(
                entry.exposed_variables(),
                variables(&[("langfuse_secret", "sk-resolved")])
            );
        }
        Scenario::MissingSetting => {
            let error = load(
                "callback_settings:\n  langfuse:\n    langfuse_secret: os.environ/CALLBACK_SECRET\n",
                &[],
            )
            .unwrap_err();
            assert!(
                matches!(error, ConfigError::MissingEnvironment { variable, path }
                if variable == "CALLBACK_SECRET" && path == "callback_settings.langfuse.langfuse_secret")
            );
        }
        Scenario::ScalarEntry(source) => {
            let error = load(source, &[]).unwrap_err();
            assert!(matches!(error, ConfigError::Yaml { .. }));
        }
        Scenario::NestedOwner => {
            let settings = load(
                "callback_settings:\n  otel:\n    endpoint: https://collector\n    optional: null\n    exporters:\n      - kind: console\n",
                &[],
            )
            .unwrap();
            let otel = settings
                .callback_settings
                .get("otel")
                .expect("otel scalars are kept");
            assert_eq!(
                otel.exposed_variables(),
                variables(&[("endpoint", "https://collector")])
            );
        }
        Scenario::OtelUnknownScalar => {
            let settings = load(
                "callback_settings:\n  otel:\n    unknown_scalar: keep-me\n",
                &[],
            )
            .unwrap();
            let otel = settings
                .callback_settings
                .get("otel")
                .expect("otel scalars are kept");
            assert_eq!(
                otel.exposed_variables(),
                variables(&[("unknown_scalar", "keep-me")])
            );
        }
        Scenario::OtelUnknownNested => {
            let error = load(
                "callback_settings:\n  otel:\n    mystery:\n      depth: 1\n",
                &[],
            )
            .unwrap_err();
            assert!(matches!(error, ConfigError::Unsupported { paths }
                if paths == ["callback_settings.otel.mystery"]));
        }
        Scenario::NestedOwnerless => {
            let error = load(
                "callback_settings:\n  custom:\n    event_types: [a]\n  generic_api:\n    endpoint: https://hook\n    headers:\n      authorization: Bearer secret\n",
                &[],
            )
            .unwrap_err();
            assert!(
                matches!(error, ConfigError::Unsupported { paths } if paths == [
                    "callback_settings.custom.event_types",
                    "callback_settings.generic_api.headers",
                ])
            );
        }
        Scenario::CallbackListShapes => {
            let one = RawCallbackList::One("one".to_owned());
            assert_eq!(one.names(), ["one"]);
            assert_eq!(one.into_names(), ["one"]);
            let many = RawCallbackList::Many(vec!["a".to_owned(), "b".to_owned()]);
            assert_eq!(many.names(), ["a", "b"]);
            assert_eq!(many.into_names(), ["a", "b"]);
        }
    }
}
