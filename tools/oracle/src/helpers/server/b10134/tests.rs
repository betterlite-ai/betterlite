#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
#[case::secrets_dropped("mcp-admin-credential-view/secrets-dropped-admin-config-kept")]
#[case::only_secrets("mcp-admin-credential-view/only-secrets-gives-null")]
#[case::empty_values("mcp-admin-credential-view/empty-values-dropped")]
#[case::no_credentials("mcp-admin-credential-view/no-credentials-no-mcp-info")]
#[case::listed("mcp-admin-credential-view/listed-server-explicit")]
#[case::legacy("mcp-admin-credential-view/legacy-public-internet")]
#[case::unregistered("mcp-admin-credential-view/listed-id-not-registered")]
fn replays_b10134_admin_credential_view_against_litellm(#[case] name: &str) {
    let root = Path::new(ORACLE_DIR);
    let source = fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case = serde_saphyr::from_str::<HelperCase>(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match));
}
