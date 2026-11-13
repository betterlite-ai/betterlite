#![cfg(test)]

use crate::loader::{ConfigError, ConfigLoader};
use rstest::rstest;
use std::collections::BTreeMap;

fn load(source: &str, environment: &[(&str, &str)]) -> Result<super::Settings, ConfigError> {
    let lookup = environment
        .iter()
        .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
        .collect::<BTreeMap<_, _>>();
    Ok(ConfigLoader { lookup }
        .load(source)?
        .sections
        .openai_workload_identity)
}

#[rstest]
#[case::loads_openai_identity_provider_id(0)]
#[case::loads_openai_service_account_id(1)]
#[case::loads_openai_identity_token_file(2)]
#[case::rejects_unrelated_key(3)]
fn openai_workload_identity_config_cases(#[case] case: u8) {
    match case {
        0 => {
            assert_eq!(load("", &[]).unwrap(), super::Settings::default());
            let loaded = load("", &[("OPENAI_IDENTITY_PROVIDER_ID", "idp_1")]).unwrap();
            assert_eq!(loaded.identity_provider_id.as_deref(), Some("idp_1"));
            assert!(loaded.service_account_id.is_none() && loaded.token_file.is_none());
            let empty = load("", &[("OPENAI_IDENTITY_PROVIDER_ID", "")]).unwrap();
            assert!(empty.identity_provider_id.is_none());
        }
        1 => {
            let loaded = load("", &[("OPENAI_SERVICE_ACCOUNT_ID", "sa_1")]).unwrap();
            assert_eq!(loaded.service_account_id.as_deref(), Some("sa_1"));
            assert!(loaded.identity_provider_id.is_none() && loaded.token_file.is_none());
            let empty = load("", &[("OPENAI_SERVICE_ACCOUNT_ID", "")]).unwrap();
            assert!(empty.service_account_id.is_none());
        }
        2 => {
            let loaded = load("", &[("OPENAI_IDENTITY_TOKEN_FILE", "/run/token")]).unwrap();
            assert_eq!(loaded.token_file.as_deref(), Some("/run/token"));
            assert!(loaded.identity_provider_id.is_none() && loaded.service_account_id.is_none());
            let empty = load("", &[("OPENAI_IDENTITY_TOKEN_FILE", "")]).unwrap();
            assert!(empty.token_file.is_none());
        }
        _ => {
            let source = "litellm_settings:\n  openai_workload_identity_unrelated_key: 1\n";
            assert!(matches!(
                load(source, &[]),
                Err(ConfigError::Unsupported { .. })
            ));
        }
    }
}
