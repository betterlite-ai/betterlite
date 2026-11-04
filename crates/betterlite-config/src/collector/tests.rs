#![cfg(test)]

use std::fs;

use super::*;

#[test]
fn loads_collector_delivery_settings() {
    let defaults = load_collector_settings(|_| None).unwrap();
    assert_eq!(defaults, CollectorSettings::default());
    let settings = load_collector_settings(|key| match key {
        "LITELLM_COLLECTOR_ENABLED" => Some("TRUE".to_owned()),
        "LITELLM_JOB_ROLE" => Some("collector".to_owned()),
        _ => None,
    })
    .unwrap();
    assert!(!(settings.enabled && settings.job_role.as_deref() != Some("collector")));
    assert!(
        load_collector_settings(|key| {
            (key == "LITELLM_COLLECTOR_BUFFER_SIZE").then(|| "0".to_owned())
        })
        .is_err()
    );
}

#[test]
fn parses_collector_addresses() {
    assert!(matches!(
        parse_collector_address("unix:///tmp/c.sock"),
        Ok(CollectorAddress::Unix { .. })
    ));
    assert!(matches!(
        parse_collector_address("tcp://localhost:123"),
        Ok(CollectorAddress::Tcp { .. })
    ));
    assert!(matches!(
        parse_collector_address("tcp://example.com:123"),
        Err(AddressError::RemoteHost(_))
    ));
}

#[test]
fn replays_the_collector_settings_corpus() {
    let bytes = fs::read("fuzz/corpus/collector_settings/defaults.yaml").unwrap();
    let settings: CollectorSettings = serde_json::from_slice(&bytes).unwrap();
    assert!(settings.validate().is_ok());
}
