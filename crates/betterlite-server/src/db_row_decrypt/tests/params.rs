use betterlite_models::decisions::Json;
use betterlite_router::deployment_identity::error::InvalidDeployment;
use serde_json::Number;

use super::support::{capture_logs, object};
use crate::db_row_decrypt::params::litellm_params;

#[test]
fn rejects_params_without_a_model() {
    let result = litellm_params(&object(r#"{"api_key":"x"}"#));
    assert!(matches!(result, Err(InvalidDeployment::MissingModel)));
}

#[test]
fn coerces_text_max_retries() {
    let params = litellm_params(&object(r#"{"model":"m","max_retries":"3"}"#)).unwrap();
    assert_eq!(
        params.get("max_retries"),
        Some(&Json::Number(Number::from(3_i64)))
    );
}

#[test]
fn drops_reserved_keys_and_nulls() {
    let params = litellm_params(&object(
        r#"{"model":"m","self":1,"params":2,"__class__":3,"api_key":null}"#,
    ))
    .unwrap();
    assert_eq!(params.get("self"), None);
    assert_eq!(params.get("params"), None);
    assert_eq!(params.get("__class__"), None);
    assert_eq!(params.get("api_key"), None);
    assert_eq!(params.get("model"), Some(&Json::Text("m".to_owned())));
}

#[test]
fn normalizes_drop_params() {
    let params = litellm_params(&object(r#"{"model":"m","drop_params":"true"}"#)).unwrap();
    assert_eq!(params.get("drop_params"), Some(&Json::Bool(true)));
    let (logs, _guard) = capture_logs();
    let unset = litellm_params(&object(r#"{"model":"m","drop_params":3}"#)).unwrap();
    assert_eq!(unset.get("drop_params"), None);
    assert!(
        logs.contents()
            .contains("is not a flag value, treating it as unset")
    );
}

#[test]
fn adds_the_false_defaults() {
    let params = litellm_params(&object(r#"{"model":"m"}"#)).unwrap();
    let flags = [
        "allow_client_keepalive_override",
        "use_in_pass_through",
        "use_litellm_proxy",
        "use_xai_oauth",
        "merge_reasoning_content_in_choices",
    ];
    assert!(
        flags
            .iter()
            .all(|flag| params.get(*flag) == Some(&Json::Bool(false)))
    );
}
