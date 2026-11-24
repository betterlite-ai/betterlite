#![cfg(test)]

use std::collections::BTreeMap;

use super::{CYBERARK_FIELDS, HASHICORP_VAULT_FIELDS, load_override_environment};

const MAPPINGS: [(&str, &str, bool); 21] = [
    ("vault_addr", "HCP_VAULT_ADDR", false),
    ("vault_token", "HCP_VAULT_TOKEN", true),
    ("approle_role_id", "HCP_VAULT_APPROLE_ROLE_ID", false),
    ("approle_secret_id", "HCP_VAULT_APPROLE_SECRET_ID", true),
    ("approle_mount_path", "HCP_VAULT_APPROLE_MOUNT_PATH", false),
    ("client_cert", "HCP_VAULT_CLIENT_CERT", false),
    ("client_key", "HCP_VAULT_CLIENT_KEY", true),
    ("vault_cert_role", "HCP_VAULT_CERT_ROLE", false),
    ("vault_namespace", "HCP_VAULT_NAMESPACE", false),
    ("vault_login_namespace", "HCP_VAULT_LOGIN_NAMESPACE", false),
    (
        "vault_secret_namespace",
        "HCP_VAULT_SECRET_NAMESPACE",
        false,
    ),
    ("vault_mount_name", "HCP_VAULT_MOUNT_NAME", false),
    ("vault_path_prefix", "HCP_VAULT_PATH_PREFIX", false),
    ("cyberark_api_base", "CYBERARK_API_BASE", false),
    ("cyberark_account", "CYBERARK_ACCOUNT", false),
    ("cyberark_username", "CYBERARK_USERNAME", false),
    ("cyberark_api_key", "CYBERARK_API_KEY", true),
    ("client_cert", "CYBERARK_CLIENT_CERT", false),
    ("client_key", "CYBERARK_CLIENT_KEY", true),
    ("ssl_verify", "CYBERARK_SSL_VERIFY", false),
    ("refresh_interval", "CYBERARK_REFRESH_INTERVAL", false),
];

#[test]
fn loads_override_environment() {
    let table = HASHICORP_VAULT_FIELDS
        .iter()
        .chain(CYBERARK_FIELDS.iter())
        .map(|field| (field.name, field.env_var, field.sensitive))
        .collect::<Vec<_>>();
    assert_eq!(table, MAPPINGS.to_vec());
    let environment = MAPPINGS
        .iter()
        .map(|(_, env_var, _)| ((*env_var).to_owned(), format!("value-of-{env_var}")))
        .collect::<BTreeMap<_, _>>();
    let expected = environment.clone();
    assert_eq!(load_override_environment(&environment), expected);
    assert!(load_override_environment(&BTreeMap::<String, String>::new()).is_empty());
    let empty = MAPPINGS
        .iter()
        .map(|(_, env_var, _)| ((*env_var).to_owned(), String::new()))
        .collect::<BTreeMap<_, _>>();
    assert!(load_override_environment(&empty).is_empty());
    let unrelated = BTreeMap::from([("HCP_VAULT_OTHER".to_owned(), "kept".to_owned())]);
    assert!(load_override_environment(&unrelated).is_empty());
}
