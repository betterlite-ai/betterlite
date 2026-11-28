#![cfg(test)]

use std::fs;
use std::path::Path;

use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, ORACLE_DIR, run_case};
use rstest::rstest;

#[rstest]
#[case::sanitize_alias("mcp-client-auth-headers/sanitize-alias")]
#[case::sanitize_alias_non_ascii("mcp-client-auth-headers/sanitize-alias-non-ascii")]
#[case::lookup_by_alias("mcp-client-auth-headers/lookup-by-alias")]
#[case::lookup_sanitized_alias("mcp-client-auth-headers/lookup-sanitized-alias")]
#[case::lookup_server_name_fallback("mcp-client-auth-headers/lookup-server-name-fallback")]
#[case::lookup_group_default("mcp-client-auth-headers/lookup-group-default")]
#[case::lookup_group_ambiguous("mcp-client-auth-headers/lookup-group-ambiguous")]
#[case::lookup_no_match("mcp-client-auth-headers/lookup-no-match")]
#[case::per_server_authorization("mcp-client-auth-headers/per-server-authorization")]
#[case::passthrough_authorization("mcp-client-auth-headers/passthrough-authorization")]
fn replays_b10155_client_auth_headers_against_litellm(#[case] name: &str) {
    let root = Path::new(ORACLE_DIR);
    let source = fs::read_to_string(root.join(format!("cases/helpers/{name}.yaml"))).unwrap();
    let case: HelperCase = serde_saphyr::from_str(&source).unwrap();
    let result = run_case(&[crate::GROUP], &case, false).unwrap();
    assert!(matches!(result, HelperRun::Match), "{name}");
}
