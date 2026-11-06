use std::collections::BTreeMap;

use betterlite_guardrails::ibm_detector_client::config::IbmDetectorConfig;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use crate::routing_mock_scripts::{ProviderCall, redact};

pub(super) fn request_values(recorded: &[ProviderCall], config: &IbmDetectorConfig) -> Vec<Value> {
    let expected_authorization = config
        .extra_headers
        .get("Authorization")
        .cloned()
        .unwrap_or_else(|| format!("Bearer {}", config.auth_token.expose_secret()));
    recorded
        .iter()
        .map(|call| {
            json!({
                "path": call.request.path,
                "body": call.request.body,
                "headers": request_headers(
                    &call.request.headers,
                    &config.extra_headers,
                    &expected_authorization,
                ),
            })
        })
        .collect()
}

fn request_headers(
    recorded: &BTreeMap<String, String>,
    extra_headers: &BTreeMap<String, String>,
    expected_authorization: &str,
) -> Value {
    let mut names: Vec<String> = vec!["content-type".to_owned(), "detector-id".to_owned()];
    for name in extra_headers.keys() {
        let lowercase = name.to_lowercase();
        if !names.contains(&lowercase) {
            names.push(lowercase);
        }
    }
    let mut selected = BTreeMap::new();
    for name in names {
        let value = recorded
            .get(&name)
            .map_or(Value::Null, |value| Value::String(value.clone()));
        selected.insert(name, value);
    }
    selected.insert(
        "authorization".to_owned(),
        authorization_value(recorded, expected_authorization),
    );
    json!(selected)
}

fn authorization_value(recorded: &BTreeMap<String, String>, expected: &str) -> Value {
    let mut probe = BTreeMap::from([("authorization".to_owned(), expected.to_owned())]);
    redact(&mut probe);
    if recorded.get("authorization") == probe.get("authorization") {
        Value::String(expected.to_owned())
    } else {
        Value::String("<mismatch>".to_owned())
    }
}
