#![cfg(test)]

use std::collections::BTreeMap;
use std::sync::Arc;

use betterlite_config::sections::otel_settings::OtelAttributes;
use betterlite_config::sections::otel_settings::Settings as OtelSettings;
use rstest::rstest;

use self::support::{catalog, catalog_inputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::legacy_metrics::filter::CallbackSettingsView;
use crate::otel::legacy_policy::config::{OpenTelemetryConfig, Signals};
use crate::otel::logger_runtime::tests::support::{fixture, fixture_disabled};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::attributes::AttributeSettings;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::{attributes, build, env, legacy};
use crate::registry::error::RegistryError;

pub(crate) mod support;

const IN_MEMORY: (&str, &str) = ("OTEL_EXPORTER", "in_memory");
const BOTH_LISTS: &str = "callback_settings:\n  otel:\n    attributes:\n      include_list:\n        - gen_ai.request.model\n      exclude_list:\n        - gen_ai.request.temperature\n";

#[test]
fn the_environment_snapshot_keeps_the_legacy_variables() {
    let inputs = catalog_inputs("", &[IN_MEMORY, ("OTEL_SERVICE_NAME", "from-env")]);
    let shared = SharedOtel::new(&inputs);
    assert_eq!(
        shared.env.get("OTEL_EXPORTER").map(String::as_str),
        Some("in_memory")
    );
    assert_eq!(
        shared.env.get("OTEL_SERVICE_NAME").map(String::as_str),
        Some("from-env")
    );
    assert_eq!(shared.env.get("LITELLM_LOGGER_NAME"), None);
}

#[rstest]
#[case::absent(None, "litellm")]
#[case::empty(Some(""), "litellm")]
#[case::named(Some("custom"), "custom")]
fn the_event_logger_name_defaults_to_litellm(
    #[case] configured: Option<&str>,
    #[case] expected: &str,
) {
    let mut environment = BTreeMap::new();
    if let Some(configured) = configured {
        environment.insert("LITELLM_LOGGER_NAME".to_owned(), configured.to_owned());
    }
    assert_eq!(env::logger_name(&environment), expected);
}

#[test]
fn prepare_carries_the_config_and_the_callback_name_into_the_provider_state() {
    let inputs = catalog_inputs("", &[IN_MEMORY]);
    let shared = SharedOtel::new(&inputs);
    let config = OpenTelemetryConfig {
        endpoint: Some("http://collector.example:4318".to_owned()),
        ..OpenTelemetryConfig::default()
    };
    let prepared =
        legacy::prepare(&inputs, &shared, config, Some("arize")).expect("the provider builds");
    assert_eq!(
        prepared.otel.config.endpoint.as_deref(),
        Some("http://collector.example:4318")
    );
    assert_eq!(prepared.otel.callback_name.as_deref(), Some("arize"));
}

#[test]
fn options_keep_the_providers_the_lifecycle_no_longer_owns() {
    let inputs = catalog_inputs("", &[IN_MEMORY]);
    let shared = SharedOtel::new(&inputs);
    let config = OpenTelemetryConfig {
        signals: Signals {
            enable_metrics: true,
            enable_events: false,
        },
        ..OpenTelemetryConfig::default()
    };
    let mut prepared =
        legacy::prepare(&inputs, &shared, config, None).expect("the providers build");
    prepared.options().expect("the options build");
    assert_eq!(shared.kept.lock().expect("unpoisoned").len(), 1);
}

#[test]
fn the_settings_view_carries_the_configured_attribute_lists() {
    let settings = OtelSettings {
        attributes: Some(OtelAttributes {
            include_list: Some(vec!["gen_ai.request.model".to_owned()]),
            exclude_list: None,
        }),
        ..OtelSettings::default()
    };
    let view = AttributeSettings::from_settings(&settings);
    let expected = OtelModelValue::Object(BTreeMap::from([(
        "include_list".to_owned(),
        OtelModelValue::Array(vec![OtelModelValue::String(
            "gen_ai.request.model".to_owned(),
        )]),
    )]));
    assert_eq!(view.otel_attributes(), Some(expected));
}

#[test]
fn a_failed_otel_build_starts_no_phoenix_logger() {
    let catalog = catalog(BOTH_LISTS, &[IN_MEMORY, ("PHOENIX_API_KEY", "phoenix-key")]);
    assert!(
        catalog
            .resolve_logger("otel")
            .expect("the name is available")
            .is_none()
    );
    assert!(catalog.existing("arize_phoenix").is_none());
    assert!(catalog.in_memory_loggers().is_empty());
}

#[test]
fn otel_without_v2_builds_the_legacy_opentelemetry() {
    let catalog = catalog("", &[IN_MEMORY]);
    let logger = catalog
        .resolve_logger("otel")
        .expect("the name is available")
        .expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), "opentelemetry");
}

#[test]
fn a_runtime_with_the_switch_off_still_builds_the_legacy_logger() {
    let fixture = fixture_disabled();
    let catalog = catalog("", &[IN_MEMORY]).with_runtime(Arc::new(fixture.runtime));
    let logger = catalog
        .resolve_logger("otel")
        .expect("the name is available")
        .expect("the legacy logger builds");
    assert_eq!(logger.callback_name(), "opentelemetry");
    assert!(fixture.constructor.records().is_empty());
}

#[test]
fn otel_with_v2_builds_through_the_runtime_constructor() {
    let fixture = fixture();
    let yaml = "callback_settings:\n  otel:\n    service_name: from-the-section\n";
    let catalog =
        catalog(yaml, &[("LITELLM_OTEL_V2", "true")]).with_runtime(Arc::new(fixture.runtime));
    let logger = catalog
        .resolve_logger("otel")
        .expect("the name is available")
        .expect("the V2 logger builds");
    assert_eq!(logger.callback_name(), "otel");
    let records = fixture.constructor.records();
    assert_eq!(records.len(), 1);
    assert_eq!(
        records.first().expect("one build").config.service_name,
        "from-the-section"
    );
    assert_eq!(catalog.in_memory_loggers().len(), 1);
}

#[test]
fn otel_with_v2_and_no_runtime_is_unavailable() {
    let catalog = catalog("", &[("LITELLM_OTEL_V2", "true")]);
    assert!(matches!(
        catalog.resolve_logger("otel"),
        Err(RegistryError::UnavailableAdapter { name }) if name == "otel"
    ));
    assert!(catalog.in_memory_loggers().is_empty());
}

#[test]
fn attributes_with_both_lists_give_an_init_error() {
    let inputs = catalog_inputs(BOTH_LISTS, &[IN_MEMORY]);
    let shared = SharedOtel::new(&inputs);
    let mut in_memory = Vec::new();
    let error = build(&inputs, None, &shared, &mut in_memory)
        .err()
        .expect("both lists are refused");
    assert!(matches!(
        error,
        CatalogError::Init { message } if message.contains("mutually exclusive")
    ));
    assert!(in_memory.is_empty());
}

#[test]
fn attributes_with_both_lists_register_no_logger() {
    let catalog = catalog(BOTH_LISTS, &[IN_MEMORY]);
    assert!(
        catalog
            .resolve_logger("otel")
            .expect("the name is available")
            .is_none()
    );
    assert!(catalog.existing("otel").is_none());
    assert!(catalog.in_memory_loggers().is_empty());
}

#[test]
fn attributes_carrying_one_list_keep_only_the_keys_that_are_set() {
    let include_only = OtelAttributes {
        include_list: Some(vec!["gen_ai.request.model".to_owned()]),
        exclude_list: None,
    };
    let value = attributes::value(Some(&include_only)).expect("a mapping");
    let OtelModelValue::Object(mapping) = value else {
        panic!("the attributes convert to a mapping");
    };
    assert_eq!(mapping.len(), 1);
    assert!(mapping.contains_key("include_list"));
    assert!(attributes::value(None).is_none());
}

#[rstest]
#[case::api_key("PHOENIX_API_KEY")]
#[case::http_endpoint("PHOENIX_COLLECTOR_HTTP_ENDPOINT")]
#[case::endpoint("PHOENIX_COLLECTOR_ENDPOINT")]
fn a_phoenix_variable_starts_the_arize_phoenix_logger(#[case] variable: &str) {
    let catalog = catalog("", &[IN_MEMORY, (variable, "https://phoenix.example")]);
    catalog
        .resolve_logger("otel")
        .expect("the name is available")
        .expect("the legacy logger builds");
    let phoenix = catalog
        .existing("arize_phoenix")
        .expect("the phoenix logger was started");
    assert_eq!(phoenix.callback_name(), "arize_phoenix");
    let names: Vec<&str> = catalog
        .in_memory_loggers()
        .iter()
        .map(|logger| logger.callback_name())
        .collect();
    assert_eq!(names, ["opentelemetry", "arize_phoenix"]);
}

#[rstest]
#[case::project_name_alone("PHOENIX_PROJECT_NAME", "a-project")]
#[case::empty_api_key("PHOENIX_API_KEY", "")]
#[case::empty_endpoint("PHOENIX_COLLECTOR_ENDPOINT", "")]
fn without_a_phoenix_variable_no_phoenix_logger_starts(
    #[case] variable: &str,
    #[case] value: &str,
) {
    let catalog = catalog("", &[IN_MEMORY, (variable, value)]);
    catalog
        .resolve_logger("otel")
        .expect("the name is available")
        .expect("the legacy logger builds");
    assert!(catalog.existing("arize_phoenix").is_none());
    assert_eq!(catalog.in_memory_loggers().len(), 1);
}

#[test]
fn another_arm_does_not_start_the_phoenix_logger() {
    let catalog = catalog("", &[IN_MEMORY, ("PHOENIX_API_KEY", "phoenix-key")]);
    catalog
        .resolve_logger("traceloop")
        .expect("the name is available")
        .expect("the legacy logger builds");
    assert!(catalog.existing("arize_phoenix").is_none());
}

#[test]
fn otel_does_not_reuse_another_arms_logger() {
    let catalog = catalog("", &[IN_MEMORY, ("LANGTRACE_API_KEY", "langtrace-key")]);
    let langtrace = catalog
        .resolve_logger("langtrace")
        .expect("the name is available")
        .expect("the langtrace logger builds");
    let otel = catalog
        .resolve_logger("otel")
        .expect("the name is available")
        .expect("the otel logger builds");
    assert!(!Arc::ptr_eq(&langtrace, &otel));
    assert_eq!(otel.callback_name(), "opentelemetry");
    assert_eq!(catalog.in_memory_loggers().len(), 2);
}

#[test]
fn phoenix_started_after_otel_with_the_switch_on_builds_the_v2_logger() {
    let fixture = fixture();
    let catalog = catalog(
        "",
        &[
            ("LITELLM_OTEL_V2", "true"),
            ("PHOENIX_API_KEY", "phoenix-key"),
        ],
    )
    .with_runtime(Arc::new(fixture.runtime));
    catalog
        .resolve_logger("otel")
        .expect("the name is available")
        .expect("the V2 logger builds");
    let names: Vec<&str> = catalog
        .in_memory_loggers()
        .iter()
        .map(|logger| logger.callback_name())
        .collect();
    assert_eq!(names, ["otel", "arize_phoenix"]);
    assert_eq!(fixture.constructor.records().len(), 2);
}
