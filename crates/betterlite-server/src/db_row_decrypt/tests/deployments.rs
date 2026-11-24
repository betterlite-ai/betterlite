use std::collections::BTreeMap;

use betterlite_db::budget_permission_rows::JsonData;
use betterlite_models::decisions::Json;
use betterlite_models::model_registration::JsonObject;
use serde_json::Number;

use super::support::{capture_logs, cipher, row, sealed, secret_lookup};
use crate::db_row_decrypt::deployments::{decrypt_model_rows, resolve_db_param};
use crate::db_row_decrypt::warnings::Warnings;

fn params_of(deployment: &JsonObject) -> &JsonObject {
    let Json::Object(params) = deployment.get("litellm_params").unwrap() else {
        panic!("a deployment carries litellm_params as an object");
    };
    params
}

fn info_of(deployment: &JsonObject) -> &JsonObject {
    let Json::Object(info) = deployment.get("model_info").unwrap() else {
        panic!("a deployment carries model_info as an object");
    };
    info
}

#[test]
fn resolves_encrypted_and_plain_params() {
    let cipher = cipher();
    let rows = [row(&format!(
        r#"{{"model_id":"dep-1","model_name":"gpt","litellm_params":{{"model":"openai/gpt-4o","api_key":"{}","api_base":"https://api.test"}},"model_info":{{"base_model":"gpt-4o"}}}}"#,
        sealed(&cipher, "sk-1")
    ))];
    let deployments = decrypt_model_rows(
        &rows,
        &cipher,
        &secret_lookup(),
        &BTreeMap::new(),
        &Warnings::default(),
    )
    .unwrap();
    assert_eq!(deployments.len(), 1);
    let deployment = &deployments[0];
    assert_eq!(
        deployment.get("model_name"),
        Some(&Json::Text("gpt".to_owned()))
    );
    let params = params_of(deployment);
    assert_eq!(
        params.get("model"),
        Some(&Json::Text("openai/gpt-4o".to_owned()))
    );
    assert_eq!(params.get("api_key"), Some(&Json::Text("sk-1".to_owned())));
    assert_eq!(
        params.get("api_base"),
        Some(&Json::Text("https://api.test".to_owned()))
    );
    assert_eq!(params.get("use_litellm_proxy"), Some(&Json::Bool(false)));
    let info = info_of(deployment);
    assert_eq!(info.get("id"), Some(&Json::Text("dep-1".to_owned())));
    assert_eq!(info.get("db_model"), Some(&Json::Bool(true)));
    assert_eq!(info.get("member_auto_router"), Some(&Json::Bool(false)));
    assert_eq!(
        info.get("base_model"),
        Some(&Json::Text("gpt-4o".to_owned()))
    );
}

#[test]
fn resolves_an_os_environ_reference_through_the_secret_lookup() {
    let rows = [row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m","api_key":"os.environ/MY_KEY"}}"#,
    )];
    let environment = BTreeMap::from([("MY_KEY".to_owned(), "sk-env".to_owned())]);
    let deployments = decrypt_model_rows(
        &rows,
        &cipher(),
        &secret_lookup(),
        &environment,
        &Warnings::default(),
    )
    .unwrap();
    assert_eq!(
        params_of(&deployments[0]).get("api_key"),
        Some(&Json::Text("sk-env".to_owned()))
    );
}

#[test]
fn a_missing_environment_variable_becomes_an_absent_param() {
    let rows = [row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m","api_key":"os.environ/ABSENT"}}"#,
    )];
    let deployments = decrypt_model_rows(
        &rows,
        &cipher(),
        &secret_lookup(),
        &BTreeMap::new(),
        &Warnings::default(),
    )
    .unwrap();
    assert_eq!(params_of(&deployments[0]).get("api_key"), None);
}

#[test]
fn a_non_string_param_is_left_alone() {
    let value = JsonData::Number(Number::from(6379_i64));
    let resolved = resolve_db_param(
        "port",
        &value,
        &cipher(),
        &secret_lookup(),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(resolved, value);
}

#[test]
fn skips_rows_with_invalid_params_and_keeps_the_rest() {
    let (logs, _guard) = capture_logs();
    let rows = [
        row(r#"{"model_id":"bad","model_name":"invalid","litellm_params":{"api_key":"x"}}"#),
        row(r#"{"model_id":"good","model_name":"gpt","litellm_params":{"model":"m"}}"#),
    ];
    let deployments = decrypt_model_rows(
        &rows,
        &cipher(),
        &secret_lookup(),
        &BTreeMap::new(),
        &Warnings::default(),
    )
    .unwrap();
    assert_eq!(deployments.len(), 1);
    assert_eq!(
        info_of(&deployments[0]).get("id"),
        Some(&Json::Text("good".to_owned()))
    );
    assert!(
        logs.contents()
            .contains("Invalid model added to proxy db. Invalid litellm params.")
    );
}

#[test]
fn normalizes_a_text_max_retries() {
    let rows = [row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m","max_retries":"3"}}"#,
    )];
    let deployments = decrypt_model_rows(
        &rows,
        &cipher(),
        &secret_lookup(),
        &BTreeMap::new(),
        &Warnings::default(),
    )
    .unwrap();
    assert_eq!(
        params_of(&deployments[0]).get("max_retries"),
        Some(&Json::Number(Number::from(3_i64)))
    );
}

#[test]
fn defaults_the_router_flags_to_false() {
    let rows = [row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m"}}"#,
    )];
    let deployments = decrypt_model_rows(
        &rows,
        &cipher(),
        &secret_lookup(),
        &BTreeMap::new(),
        &Warnings::default(),
    )
    .unwrap();
    let params = params_of(&deployments[0]);
    assert_eq!(
        params.get("allow_client_keepalive_override"),
        Some(&Json::Bool(false))
    );
    assert_eq!(params.get("use_in_pass_through"), Some(&Json::Bool(false)));
    assert_eq!(params.get("use_litellm_proxy"), Some(&Json::Bool(false)));
    assert_eq!(params.get("use_xai_oauth"), Some(&Json::Bool(false)));
    assert_eq!(
        params.get("merge_reasoning_content_in_choices"),
        Some(&Json::Bool(false))
    );
}

#[test]
fn drops_reserved_and_null_params_and_normalizes_drop_params() {
    let rows = [row(
        r#"{"model_id":"dep-1","model_name":"gpt","litellm_params":{"model":"m","self":1,"params":2,"__class__":3,"empty":null,"drop_params":"true"}}"#,
    )];
    let deployments = decrypt_model_rows(
        &rows,
        &cipher(),
        &secret_lookup(),
        &BTreeMap::new(),
        &Warnings::default(),
    )
    .unwrap();
    let params = params_of(&deployments[0]);
    assert_eq!(params.get("self"), None);
    assert_eq!(params.get("params"), None);
    assert_eq!(params.get("__class__"), None);
    assert_eq!(params.get("empty"), None);
    assert_eq!(params.get("drop_params"), Some(&Json::Bool(true)));
}
