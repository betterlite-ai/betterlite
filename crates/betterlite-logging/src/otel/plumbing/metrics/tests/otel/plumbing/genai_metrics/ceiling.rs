use std::collections::BTreeMap;

use betterlite_logging::otel::plumbing::metrics::attributes::{
    BOUNDED_HIDDEN_PARAM_KEYS, METRIC_ATTRIBUTE_CEILING, bounded_attributes, common_attributes,
};

use super::support::{DURATION, attributes_of, kwargs, late_settings, record, record_with};

#[test]
fn the_ceiling_is_the_twelve_sorted_litellm_names() {
    assert_eq!(
        METRIC_ATTRIBUTE_CEILING,
        [
            "gen_ai.framework",
            "gen_ai.operation.name",
            "gen_ai.provider.name",
            "gen_ai.request.model",
            "gen_ai.system",
            "hidden_params",
            "metadata.user_api_key_alias",
            "metadata.user_api_key_hash",
            "metadata.user_api_key_org_id",
            "metadata.user_api_key_team_alias",
            "metadata.user_api_key_team_id",
            "metadata.user_api_key_user_id",
        ]
    );
    let mut sorted = METRIC_ATTRIBUTE_CEILING;
    sorted.sort_unstable();
    assert_eq!(sorted, METRIC_ATTRIBUTE_CEILING);
}

#[test]
fn only_the_model_id_hidden_param_key_is_bounded() {
    assert_eq!(BOUNDED_HIDDEN_PARAM_KEYS, ["model_id"]);
}

#[test]
fn the_ceiling_drops_metadata_keys_the_common_attributes_built() {
    let extra = r#"{"standard_logging_object": {"metadata": {
        "user_api_key_alias": "key-a",
        "user_api_key_user_email": "a@example.com",
        "requester_metadata": {"team": 1}
    }}}"#;
    let call_kwargs = kwargs(extra);
    let unbounded = common_attributes(&call_kwargs).expect("attributes");
    assert!(
        unbounded
            .iter()
            .any(|(key, _)| key == "metadata.user_api_key_user_email")
    );
    let bounded = bounded_attributes(&call_kwargs).expect("attributes");
    let bounded_keys: Vec<&str> = bounded.iter().map(|(key, _)| key.as_str()).collect();
    assert!(bounded_keys.contains(&"metadata.user_api_key_alias"));
    assert!(!bounded_keys.contains(&"metadata.user_api_key_user_email"));
    assert!(!bounded_keys.contains(&"metadata.requester_metadata"));
}

#[test]
fn the_recorder_labels_only_ceiling_attributes() {
    let extra = r#"{
        "standard_logging_object": {
            "metadata": {
                "user_api_key_alias": "key-a",
                "user_api_key_user_email": "a@example.com",
                "requester_ip_address": "10.0.0.1"
            },
            "hidden_params": {
                "model_id": "deployment-1",
                "api_base": "https://x",
                "cache_key": "c"
            }
        }
    }"#;
    let recorded = record(extra, None);
    let expected = BTreeMap::from([
        ("gen_ai.framework".to_owned(), "litellm".to_owned()),
        ("gen_ai.operation.name".to_owned(), "chat".to_owned()),
        ("gen_ai.provider.name".to_owned(), "openai".to_owned()),
        ("gen_ai.request.model".to_owned(), "gpt-4o".to_owned()),
        ("gen_ai.system".to_owned(), "openai".to_owned()),
        (
            "hidden_params".to_owned(),
            r#"{"model_id":"deployment-1"}"#.to_owned(),
        ),
        ("metadata.user_api_key_alias".to_owned(), "key-a".to_owned()),
    ]);
    assert_eq!(attributes_of(&recorded, DURATION), expected);
}

#[test]
fn a_hidden_params_model_id_that_is_null_makes_no_label() {
    let extra = r#"{"standard_logging_object": {"hidden_params": {"model_id": null, "api_base": "https://x"}}}"#;
    let recorded = record(extra, None);
    assert!(!attributes_of(&recorded, DURATION).contains_key("hidden_params"));
}

#[test]
fn an_include_list_cannot_widen_the_ceiling() {
    let settings = late_settings(Some(
        r#"{"include_list": ["gen_ai.system", "metadata.user_api_key_user_email"]}"#,
    ));
    let extra = r#"{"standard_logging_object": {"metadata": {"user_api_key_user_email": "a@example.com"}}}"#;
    let recorded = record_with(settings, None, extra, None);
    let expected = BTreeMap::from([("gen_ai.system".to_owned(), "openai".to_owned())]);
    assert_eq!(attributes_of(&recorded, DURATION), expected);
}
