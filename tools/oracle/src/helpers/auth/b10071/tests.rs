#![cfg(test)]

use std::fs;
use std::path::PathBuf;

use betterlite_oracle_helper_api::helper::Helper as _;
use betterlite_oracle_runner::case::helper::HelperCase;
use betterlite_oracle_runner::helpers::{HelperRun, run_case};
use rstest::rstest;
use serde_json::{Value, json};

use super::Op;

fn run(input: &Value) -> Value {
    Op::decode_and_run(input).unwrap()
}

#[test]
fn a_proxy_admin_passes_the_gate() {
    assert_eq!(
        run(&json!({"function": "gate", "user_role": "proxy_admin", "operation": "create"})),
        json!({"passed": true, "status": null})
    );
}

#[rstest]
#[case::viewer("proxy_admin_viewer", "create")]
#[case::org_admin("org_admin", "delete")]
#[case::internal_user("internal_user", "update")]
#[case::internal_user_viewer("internal_user_viewer", "list")]
fn a_role_that_is_not_the_proxy_admin_is_refused(#[case] role: &str, #[case] operation: &str) {
    assert_eq!(
        run(&json!({"function": "gate", "user_role": role, "operation": operation})),
        json!({"passed": false, "status": 403})
    );
}

#[test]
fn a_caller_without_a_role_is_refused() {
    assert_eq!(
        run(&json!({"function": "gate", "user_role": null, "operation": "create"})),
        json!({"passed": false, "status": 403})
    );
}

#[rstest]
#[case::index_itself("DELETE", "/azure_ai/indexes/my-index", "my-index", true)]
#[case::document("DELETE", "/azure_ai/indexes/my-index/docs", "my-index", false)]
#[case::service_create("POST", "/azure_ai/indexes", "my-index", true)]
#[case::read("GET", "/azure_ai/indexes/my-index", "my-index", false)]
fn the_lifecycle_adapter_returns_the_litellm_answer(
    #[case] method: &str,
    #[case] path: &str,
    #[case] index_name: &str,
    #[case] expected: bool,
) {
    let input = json!({
        "function": "lifecycle",
        "method": method,
        "path": path,
        "index_name": index_name,
    });
    assert_eq!(run(&input), json!(expected));
}

#[rstest]
fn gate_and_lifecycle_cases_match_litellm(
    #[base_dir = "../../.."]
    #[files("cases/helpers/vector-index-admin/*.yaml")]
    path: PathBuf,
) {
    let source = fs::read_to_string(path).expect("case file");
    let case: HelperCase = serde_saphyr::from_str(&source).expect("helper case");
    assert!(matches!(
        run_case(&[crate::GROUP], &case, false).expect("case runs"),
        HelperRun::Match
    ));
}
