#![cfg(test)]

use std::collections::BTreeMap;

use crate::otel::config::{ExporterOwner, ExporterSpec};

use super::{credential_gated_exporters, ensure_mappers, is_unconfigured_placeholder};

fn placeholder() -> ExporterSpec {
    ExporterSpec {
        kind: "console".to_owned(),
        endpoint: None,
        traces_endpoint: None,
        headers: None,
        owner: None,
        options: None,
        use_simple_processor: None,
        requires_headers: false,
    }
}

fn configured() -> ExporterSpec {
    let mut spec = placeholder();
    spec.kind = "otlp_http".to_owned();
    spec.endpoint = Some("https://collector.example/v1/traces".to_owned());
    spec
}

#[test]
fn ensure_mappers_appends_the_missing_names_in_order() {
    let names = vec!["genai".to_owned(), "weave".to_owned()];
    assert_eq!(
        ensure_mappers(&names, &["openinference", "weave", "langtrace"]),
        vec![
            "genai".to_owned(),
            "weave".to_owned(),
            "openinference".to_owned(),
            "langtrace".to_owned()
        ]
    );
}

#[test]
fn ensure_mappers_keeps_names_when_every_name_exists() {
    let names = vec!["genai".to_owned(), "openinference".to_owned()];
    assert_eq!(
        ensure_mappers(&names, &["openinference"]),
        vec!["genai".to_owned(), "openinference".to_owned()]
    );
}

#[test]
fn a_default_spec_is_the_unconfigured_placeholder() {
    assert!(is_unconfigured_placeholder(&placeholder()));
}

#[test]
fn a_configured_spec_is_not_the_placeholder() {
    assert!(!is_unconfigured_placeholder(&configured()));
    assert!(!is_unconfigured_placeholder(&ExporterSpec {
        kind: "in_memory".to_owned(),
        ..placeholder()
    }));
    assert!(!is_unconfigured_placeholder(&ExporterSpec {
        endpoint: Some("https://collector.example/v1/traces".to_owned()),
        ..placeholder()
    }));
    assert!(!is_unconfigured_placeholder(&ExporterSpec {
        traces_endpoint: Some("https://collector.example/v1/traces".to_owned()),
        ..placeholder()
    }));
    assert!(!is_unconfigured_placeholder(&ExporterSpec {
        headers: Some("x=1".to_owned()),
        ..placeholder()
    }));
    assert!(!is_unconfigured_placeholder(&ExporterSpec {
        owner: Some(ExporterOwner::ArizeAx),
        ..placeholder()
    }));
    assert!(!is_unconfigured_placeholder(&ExporterSpec {
        options: Some(BTreeMap::from([("key".to_owned(), "value".to_owned())])),
        ..placeholder()
    }));
    assert!(!is_unconfigured_placeholder(&ExporterSpec {
        use_simple_processor: Some(true),
        ..placeholder()
    }));
    assert!(!is_unconfigured_placeholder(&ExporterSpec {
        requires_headers: true,
        ..placeholder()
    }));
}

#[test]
fn credential_gating_drops_placeholders_and_appends_the_gated_spec() {
    let gated =
        credential_gated_exporters(vec![placeholder(), configured()], ExporterOwner::WeaveOtel);
    assert_eq!(
        gated,
        [
            configured(),
            ExporterSpec {
                kind: "console".to_owned(),
                endpoint: None,
                traces_endpoint: None,
                headers: None,
                owner: Some(ExporterOwner::WeaveOtel),
                options: None,
                use_simple_processor: None,
                requires_headers: true,
            }
        ]
    );
}

#[test]
fn credential_gating_keeps_an_existing_gated_spec() {
    let first = credential_gated_exporters(vec![placeholder()], ExporterOwner::WeaveOtel);
    let second = credential_gated_exporters(first, ExporterOwner::WeaveOtel);
    assert_eq!(second.len(), 2);
    assert!(
        second
            .iter()
            .all(|spec| spec.owner == Some(ExporterOwner::WeaveOtel))
    );
}
