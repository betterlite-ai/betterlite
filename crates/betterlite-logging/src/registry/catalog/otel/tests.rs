#![cfg(test)]

use std::sync::Arc;

use betterlite_config::sections::otel_settings::OtelAttributes;
use rstest::rstest;

use self::support::{catalog, catalog_inputs};
use crate::callback_logger::CallbackLogger;
use crate::otel::logger_runtime::tests::support::{fixture, fixture_disabled};
use crate::otel::model::utils::coercion::OtelModelValue;
use crate::registry::catalog::CatalogError;
use crate::registry::catalog::otel::shared::SharedOtel;
use crate::registry::catalog::otel::{attributes, build};
use crate::registry::error::RegistryError;

pub(crate) mod support;

const IN_MEMORY: (&str, &str) = ("OTEL_EXPORTER", "in_memory");
const BOTH_LISTS: &str = "callback_settings:\n  otel:\n    attributes:\n      include_list:\n        - gen_ai.request.model\n      exclude_list:\n        - gen_ai.request.temperature\n";

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
