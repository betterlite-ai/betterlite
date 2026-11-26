use std::collections::BTreeMap;

use betterlite_models::controls::call_params::TRUSTED_CREDENTIALS_KEY;
use secrecy::ExposeSecret as _;

use crate::api::file_credentials::accounting::{
    AccountingError, add_deployment_model_info, add_internal_model_credentials,
};

use super::support::{Catalog, Log, LookupCall, credentials, lookup_calls, object};

#[test]
fn deployment_accounting_hints_stamp_the_resolved_model_info() {
    let mut catalog = Catalog::new(&Log::default());
    catalog.resolved = credentials(&[(
        "d1",
        r#"{"id":"d1","db_model":false,"member_auto_router":false,"team_id":"t"}"#,
    )]);
    let mut data = object(r#"{"x":1,"litellm_metadata":{"keep":1,"model_info":{"old":true}}}"#);
    add_deployment_model_info(&mut data, Some(&catalog), "d1").unwrap();
    assert_eq!(
        data,
        object(
            r#"{"x":1,"litellm_metadata":{"keep":1,"model_info":{"id":"d1","db_model":false,"member_auto_router":false,"team_id":"t"}}}"#
        )
    );
}

#[test]
fn deployment_accounting_hints_keep_data_when_no_deployment_resolves() {
    let catalog = Catalog::default();
    let mut data = object(r#"{"x":1}"#);
    add_deployment_model_info(&mut data, Some(&catalog), "missing").unwrap();
    assert_eq!(data, object(r#"{"x":1}"#));
    add_deployment_model_info(&mut data, None::<&Catalog>, "missing").unwrap();
    assert_eq!(data, object(r#"{"x":1}"#));
}

#[test]
fn deployment_accounting_hints_reject_non_object_metadata() {
    let mut catalog = Catalog::new(&Log::default());
    catalog.resolved = credentials(&[("d1", r#"{"id":"d1"}"#)]);
    let mut data = object(r#"{"litellm_metadata":"text"}"#);
    let failure = add_deployment_model_info(&mut data, Some(&catalog), "d1").unwrap_err();
    assert!(matches!(failure, AccountingError::MetadataNotObject));
    assert_eq!(data, object(r#"{"litellm_metadata":"text"}"#));
}

#[test]
fn deployment_accounting_hints_internal_credentials_install_the_snapshot() {
    let events = Log::default();
    let mut catalog = Catalog::new(&events);
    catalog.credentials = credentials(&[(
        "d1",
        r#"{"api_key":"k1","api_base":"https://one","vertex_credentials":{"type":"service_account"},"model":"m1"}"#,
    )]);
    let mut data = object(r#"{"x":1}"#);
    let snapshot = add_internal_model_credentials(&mut data, Some(&catalog), Some("d1")).unwrap();
    let values: BTreeMap<&str, &str> = snapshot
        .values
        .iter()
        .map(|(name, secret)| (name.as_str(), secret.expose_secret()))
        .collect();
    assert_eq!(
        values,
        BTreeMap::from([
            ("api_base", "https://one"),
            ("api_key", "k1"),
            ("model", "m1"),
        ])
    );
    assert_eq!(data, object(r#"{"x":1}"#));
    assert_eq!(
        lookup_calls(&catalog),
        [LookupCall {
            model_id: "d1".to_owned(),
            team_id: None,
        }]
    );
}

#[test]
fn deployment_accounting_hints_internal_credentials_skip_missing_inputs() {
    let events = Log::default();
    let mut catalog = Catalog::new(&events);
    catalog.credentials = credentials(&[("d1", r#"{"api_key":"k1"}"#)]);
    let mut data = object(&format!(
        r#"{{"x":1,"{TRUSTED_CREDENTIALS_KEY}":{{"api_key":"caller"}}}}"#
    ));
    let before = data.clone();
    assert!(add_internal_model_credentials(&mut data, Some(&catalog), None).is_none());
    assert!(add_internal_model_credentials(&mut data, None::<&Catalog>, Some("d1")).is_none());
    assert!(add_internal_model_credentials(&mut data, Some(&catalog), Some("missing")).is_none());
    assert_eq!(data, before);
    // the None model and the None catalog never reach the port
    assert_eq!(
        lookup_calls(&catalog),
        [LookupCall {
            model_id: "missing".to_owned(),
            team_id: None,
        }]
    );
}

#[test]
fn deployment_accounting_hints_internal_credentials_swallow_a_lookup_error() {
    let mut failing = Catalog::new(&Log::default());
    failing.fail_lookup = true;
    let mut data = object(r#"{"x":1}"#);
    let before = data.clone();
    assert!(add_internal_model_credentials(&mut data, Some(&failing), Some("d1")).is_none());
    assert_eq!(data, before);
    assert_eq!(lookup_calls(&failing).len(), 1);
}

#[test]
fn deployment_accounting_hints_internal_credentials_replace_a_caller_snapshot() {
    let mut catalog = Catalog::new(&Log::default());
    catalog.credentials = credentials(&[("d1", r#"{"api_key":"k1"}"#)]);
    let mut data = object(&format!(
        r#"{{"x":1,"{TRUSTED_CREDENTIALS_KEY}":{{"api_key":"caller"}}}}"#
    ));
    let snapshot = add_internal_model_credentials(&mut data, Some(&catalog), Some("d1")).unwrap();
    assert_eq!(data, object(r#"{"x":1}"#));
    assert_eq!(snapshot.values.len(), 1);
    assert_eq!(
        snapshot.values.get("api_key").unwrap().expose_secret(),
        "k1"
    );
}
