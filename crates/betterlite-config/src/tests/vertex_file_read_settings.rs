use std::collections::BTreeMap;

use secrecy::ExposeSecret;

const SERVICE_ACCOUNT_ENV: &str = "GCS_PATH_SERVICE_ACCOUNT";
const SERVICE_ACCOUNT_PATH: &str = "/run/secrets/vertex-service-account.json";

fn loaded_value(config: &crate::types::Config) -> Option<&str> {
    config
        .environment_variables
        .get(SERVICE_ACCOUNT_ENV)
        .and_then(Option::as_ref)
        .map(ExposeSecret::expose_secret)
}

#[test]
fn loads_gcs_path_service_account() {
    let empty = super::map_loader(BTreeMap::new()).load("").unwrap();
    assert_eq!(loaded_value(&empty), None);
    let literal = super::map_loader(BTreeMap::new())
        .load(
            "environment_variables:\n  GCS_PATH_SERVICE_ACCOUNT: /run/secrets/vertex-service-account.json\n",
        )
        .unwrap();
    assert_eq!(loaded_value(&literal), Some(SERVICE_ACCOUNT_PATH));
    let mut values = BTreeMap::new();
    values.insert(
        "VERTEX_SERVICE_ACCOUNT".to_owned(),
        SERVICE_ACCOUNT_PATH.to_owned(),
    );
    let referenced = super::map_loader(values)
        .load("environment_variables:\n  GCS_PATH_SERVICE_ACCOUNT: os.environ/VERTEX_SERVICE_ACCOUNT\n")
        .unwrap();
    assert_eq!(loaded_value(&referenced), Some(SERVICE_ACCOUNT_PATH));
    assert!(!format!("{referenced:?}").contains(SERVICE_ACCOUNT_PATH));
}
