use std::collections::BTreeMap;

use betterlite_models::decisions::Json;
use indexmap::IndexMap;

use super::support;
use crate::datadog::cost_management::datadog_cost_management::tags::extract_tags;

fn tags_with(payload: &support::Raw) -> IndexMap<String, String> {
    let settings = support::settings(&support::init(&[]));
    extract_tags(
        &settings.environment,
        &settings.cost_tag_keys,
        &support::payload(&payload.0),
    )
}

#[test]
fn tags_carry_the_environment_base_tags_and_payload_focus_dimensions() {
    let tags = tags_with(&support::default_payload());
    assert_eq!(support::tag(&tags, "env"), Some("prod"));
    assert_eq!(support::tag(&tags, "service"), Some("gateway"));
    assert_eq!(support::tag(&tags, "host"), Some("host-1"));
    assert_eq!(support::tag(&tags, "pod_name"), Some("pod-1"));
    assert_eq!(support::tag(&tags, "provider"), Some("openai"));
    assert_eq!(support::tag(&tags, "model"), Some("gpt-4o"));
    assert_eq!(support::tag(&tags, "model_id"), Some("router-1"));
}

#[test]
fn tags_use_the_datadog_environment_defaults() {
    let settings = support::settings_with(
        &support::init(&[]),
        &BTreeMap::from([
            ("DD_API_KEY".to_owned(), "api-key".to_owned()),
            ("DD_APP_KEY".to_owned(), "app-key".to_owned()),
        ]),
    );
    let payload = support::default_payload().build();
    let tags = extract_tags(
        &settings.environment,
        &settings.cost_tag_keys,
        payload.as_ref(),
    );
    assert_eq!(support::tag(&tags, "env"), Some("unknown"));
    assert_eq!(support::tag(&tags, "service"), Some("litellm-server"));
    assert_eq!(support::tag(&tags, "host"), Some(""));
    assert_eq!(support::tag(&tags, "pod_name"), Some("unknown"));
}

#[test]
fn tags_omit_a_missing_provider_and_model_id() {
    let payload = support::default_payload()
        .set("/custom_llm_provider", Json::Null)
        .set("/model_id", Json::Null);
    let tags = tags_with(&payload);
    assert_eq!(support::tag(&tags, "provider"), None);
    assert_eq!(support::tag(&tags, "model_id"), None);
    assert_eq!(support::tag(&tags, "model"), Some("gpt-4o"));
}
