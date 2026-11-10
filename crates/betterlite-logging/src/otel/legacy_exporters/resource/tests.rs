#![cfg(test)]
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::sync::{Arc, Mutex};

use opentelemetry_sdk::Resource;
use rstest::rstest;
use tracing::field::{Field, Visit};
use tracing::subscriber::with_default;
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::layer::{Context, Layer};

use super::legacy_resource;
use crate::otel::legacy_policy::config::{Exporter, OpenTelemetryConfig, Signals};

fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect()
}

fn config(service_name: &str, deployment_environment: &str, model_id: &str) -> OpenTelemetryConfig {
    OpenTelemetryConfig {
        exporter: Exporter::Console,
        endpoint: None,
        headers: None,
        signals: Signals::default(),
        service_name: service_name.to_owned(),
        deployment_environment: deployment_environment.to_owned(),
        model_id: model_id.to_owned(),
        ignore_context_propagation: false,
        skip_set_global: false,
        capture_message_content: None,
        semconv_stability_opt_in: BTreeSet::new(),
        baggage_team_metadata_keys: Vec::new(),
        baggage_metadata_keys: Vec::new(),
        attributes: None,
    }
}

fn attributes(resource: &Resource) -> BTreeMap<String, String> {
    resource
        .iter()
        .map(|(key, value)| (key.as_str().to_owned(), value.as_str().into_owned()))
        .collect()
}

#[test]
fn base_attributes_come_from_the_config() {
    let resource = legacy_resource(&config("checkout", "staging", "gpt-4o"), &BTreeMap::new());
    assert_eq!(
        attributes(&resource),
        env(&[
            ("service.name", "checkout"),
            ("deployment.environment", "staging"),
            ("model_id", "gpt-4o"),
        ])
    );
}

#[test]
fn model_id_falls_back_to_the_service_name() {
    let resource = legacy_resource(&config("checkout", "staging", ""), &BTreeMap::new());
    assert_eq!(
        attributes(&resource).get("model_id").map(String::as_str),
        Some("checkout")
    );
}

#[rstest]
#[case::empty_service_name("", "staging", "gpt-4o", &["deployment.environment", "model_id"])]
#[case::empty_environment("checkout", "", "gpt-4o", &["model_id", "service.name"])]
#[case::empty_everything("", "", "", &[])]
fn empty_config_values_are_not_written(
    #[case] service_name: &str,
    #[case] deployment_environment: &str,
    #[case] model_id: &str,
    #[case] expected: &[&str],
) {
    let resource = legacy_resource(
        &config(service_name, deployment_environment, model_id),
        &BTreeMap::new(),
    );
    let names: Vec<String> = attributes(&resource).into_keys().collect();
    assert_eq!(names, expected);
}

#[test]
fn an_empty_service_name_leaves_no_model_id() {
    let resource = legacy_resource(&config("", "", ""), &BTreeMap::new());
    assert!(!attributes(&resource).contains_key("model_id"));
}

#[test]
fn environment_attributes_override_the_base_attributes() {
    let resource = legacy_resource(
        &config("checkout", "staging", "gpt-4o"),
        &env(&[(
            "OTEL_RESOURCE_ATTRIBUTES",
            "service.name=env-service,deployment.environment=prod,model_id=env-model",
        )]),
    );
    assert_eq!(
        attributes(&resource),
        env(&[
            ("service.name", "env-service"),
            ("deployment.environment", "prod"),
            ("model_id", "env-model"),
        ])
    );
}

#[test]
fn environment_attributes_add_unknown_names() {
    let resource = legacy_resource(
        &config("checkout", "staging", "gpt-4o"),
        &env(&[("OTEL_RESOURCE_ATTRIBUTES", "cloud.region=eu-west-1")]),
    );
    let names = attributes(&resource);
    assert_eq!(
        names.get("cloud.region").map(String::as_str),
        Some("eu-west-1")
    );
    assert_eq!(
        names.get("service.name").map(String::as_str),
        Some("checkout")
    );
}

#[test]
fn resource_attribute_names_and_values_are_trimmed() {
    let resource = legacy_resource(
        &config("checkout", "staging", "gpt-4o"),
        &env(&[("OTEL_RESOURCE_ATTRIBUTES", " cloud.region = eu-west-1 ")]),
    );
    assert_eq!(
        attributes(&resource)
            .get("cloud.region")
            .map(String::as_str),
        Some("eu-west-1")
    );
}

#[test]
fn resource_attribute_values_are_percent_decoded() {
    let resource = legacy_resource(
        &config("checkout", "staging", "gpt-4o"),
        &env(&[(
            "OTEL_RESOURCE_ATTRIBUTES",
            "service.namespace=my%20team%2Fcheckout",
        )]),
    );
    assert_eq!(
        attributes(&resource)
            .get("service.namespace")
            .map(String::as_str),
        Some("my team/checkout")
    );
}

#[test]
fn service_name_environment_variable_wins_over_resource_attributes() {
    let resource = legacy_resource(
        &config("checkout", "staging", "gpt-4o"),
        &env(&[
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "service.name=from-attributes,elsewhere=1",
            ),
            ("OTEL_SERVICE_NAME", "from-service-name"),
        ]),
    );
    let names = attributes(&resource);
    assert_eq!(
        names.get("service.name").map(String::as_str),
        Some("from-service-name")
    );
    assert_eq!(names.get("elsewhere").map(String::as_str), Some("1"));
}

#[rstest]
#[case::empty_service_name("OTEL_SERVICE_NAME", "")]
#[case::empty_attributes("OTEL_RESOURCE_ATTRIBUTES", "")]
fn an_empty_environment_value_is_ignored(#[case] name: &str, #[case] value: &str) {
    let resource = legacy_resource(
        &config("checkout", "staging", "gpt-4o"),
        &env(&[(name, value)]),
    );
    assert_eq!(
        attributes(&resource)
            .get("service.name")
            .map(String::as_str),
        Some("checkout")
    );
}

#[test]
fn the_resource_carries_no_sdk_attributes() {
    let resource = legacy_resource(&config("checkout", "staging", "gpt-4o"), &BTreeMap::new());
    assert!(
        attributes(&resource)
            .keys()
            .all(|name| !name.starts_with("telemetry.sdk."))
    );
}

#[derive(Clone, Default)]
struct Warnings(Arc<Mutex<Vec<String>>>);

impl<S: Subscriber> Layer<S> for Warnings {
    fn on_event(&self, event: &Event<'_>, _context: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        if let Ok(mut events) = self.0.lock() {
            events.push(fields.0.join(" "));
        }
    }
}

#[derive(Default)]
struct Fields(Vec<String>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.push(format!("{}={value}", field.name()));
    }
}

#[test]
fn every_unparsable_resource_attribute_pair_is_reported() {
    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = tracing_subscriber::registry().with(Warnings(Arc::clone(&events)));
    let resource = with_default(subscriber, || {
        legacy_resource(
            &config("checkout", "staging", "gpt-4o"),
            &env(&[(
                "OTEL_RESOURCE_ATTRIBUTES",
                "a=1,broken-entry,=no-name,c=3,trailing",
            )]),
        )
    });
    assert_eq!(
        attributes(&resource).keys().cloned().collect::<Vec<_>>(),
        vec![
            "a".to_owned(),
            "c".to_owned(),
            "deployment.environment".to_owned(),
            "model_id".to_owned(),
            "service.name".to_owned(),
        ]
    );
    let captured = events.lock().expect("events");
    assert_eq!(captured.len(), 3);
    assert!(captured.iter().any(|line| line.contains("position=2")));
    assert!(captured.iter().any(|line| line.contains("position=3")));
    assert!(captured.iter().any(|line| line.contains("position=5")));
    drop(captured);
}
