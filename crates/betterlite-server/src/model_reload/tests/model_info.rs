use betterlite_models::decisions::Json;

use crate::model_reload::model_info::{expects_liveness_on_this_pod, model_info_as_mapping};

use super::support::{object, value};

#[test]
fn model_info_mapping_variants() {
    let stored = r#"{"id":"a","db_model":true}"#;
    assert_eq!(model_info_as_mapping(&value(stored)), Some(object(stored)));
    assert_eq!(
        model_info_as_mapping(&Json::Text(stored.to_owned())),
        Some(object(stored))
    );
    assert_eq!(model_info_as_mapping(&Json::Text("[1]".to_owned())), None);
    assert_eq!(model_info_as_mapping(&Json::Text("7".to_owned())), None);
    assert_eq!(model_info_as_mapping(&Json::Text("null".to_owned())), None);
    assert_eq!(
        model_info_as_mapping(&Json::Text(r#""a string""#.to_owned())),
        None
    );
    assert_eq!(
        model_info_as_mapping(&Json::Text("not json".to_owned())),
        None
    );
    assert_eq!(model_info_as_mapping(&Json::Null), None);
    assert_eq!(model_info_as_mapping(&Json::Bool(true)), None);
    assert_eq!(
        model_info_as_mapping(&Json::Number(serde_json::Number::from(7_i64))),
        None
    );
    assert_eq!(model_info_as_mapping(&Json::List(Vec::new())), None);
}

#[test]
fn liveness_follows_the_environment_rule() {
    let gated = value(r#"{"supported_environments":["development","staging"]}"#);
    assert!(expects_liveness_on_this_pod(&gated, Some("staging")));
    assert!(!expects_liveness_on_this_pod(&gated, Some("production")));
    assert!(!expects_liveness_on_this_pod(
        &value(r#"{"supported_environments":[]}"#),
        Some("production")
    ));
    assert!(expects_liveness_on_this_pod(
        &value(r#"{"supported_environments":null}"#),
        None
    ));
    assert!(expects_liveness_on_this_pod(
        &value(r#"{"id":"a"}"#),
        Some("production")
    ));
}

#[test]
fn an_unreadable_or_refused_rule_still_expects_liveness() {
    assert!(expects_liveness_on_this_pod(
        &value(r#""not an object""#),
        Some("production")
    ));
    assert!(expects_liveness_on_this_pod(
        &value("null"),
        Some("production")
    ));
    assert!(expects_liveness_on_this_pod(
        &value(r#"{"supported_environments":"production"}"#),
        Some("production")
    ));
    assert!(expects_liveness_on_this_pod(
        &value(r#"{"supported_environments":[1]}"#),
        Some("production")
    ));
    assert!(expects_liveness_on_this_pod(
        &value(r#"{"supported_environments":["production"]}"#),
        None
    ));
    assert!(expects_liveness_on_this_pod(
        &value(r#"{"supported_environments":["production"]}"#),
        Some("qa")
    ));
}
