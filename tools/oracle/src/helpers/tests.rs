#![cfg(test)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use super::GROUPS;
use crate::case::parse_case;
use betterlite_oracle_runner::case::helper::{HelperCase, validate_helper};
use betterlite_oracle_runner::helpers::{HelperCall, HelperRun, invoke, run_case};

#[rstest::rstest]
#[case::unknown_operation("call: {nope: {}}", "unknown variant")]
#[case::unknown_input_field("call: {url_path: {x: 1}}", "unknown field")]
#[case::missing_field_name("call: {url_path: {function: segment, value: a}}", "missing field")]
#[case::unknown_case_field("x: 1", "unknown field")]
#[case::duplicate_field("name: b", "duplicate")]
#[case::unknown_dot_paths_function("call: {dot_paths: {nope: {}}}", "unknown variant")]
#[case::unknown_dot_paths_field(
    "call: {dot_paths: {delete: {data: {}, path: a, x: 1}}}",
    "unknown field"
)]
#[case::missing_dot_paths_path("call: {dot_paths: {delete: {data: {}}}}", "missing field")]
fn helper_cases_reject_unknown_and_duplicate_fields(#[case] fields: &str, #[case] reason: &str) {
    let source = format!("kind: helper\nname: a\nenv: {{}}\ncompare: {{ignore: []}}\n{fields}\n");
    let error = parse_case(Path::new("<helper>"), &source).unwrap_err();
    assert!(error.to_string().contains(reason));
}

#[test]
fn management_helper_case_rejects_unknown_fields() {
    let source = "kind: helper\nname: invalid\ncall: {management_oracle: {domain: models, operation: {path_encoding: {function: segment, value: a, field_name: id, extra: true}}}}\nenv: {}\ncompare: {ignore: []}\n";
    let error = parse_case(Path::new("<helper>"), source).unwrap_err();
    assert!(error.to_string().contains("unknown field"));
}

#[rstest::rstest]
#[case::empty_name("''", "/a", "name cannot be empty")]
#[case::relative_pointer("a", "a", "JSON pointers")]
fn helper_cases_are_validated(#[case] name: &str, #[case] pointer: &str, #[case] reason: &str) {
    let source = format!(
        "kind: helper\nname: {name}\ncall: {{url_path: {{function: segment, value: a, field_name: id}}}}\nenv: {{}}\ncompare: {{ignore: [{{pointer: '{pointer}', why: w}}]}}\n"
    );
    assert!(
        serde_saphyr::from_str(&source)
            .map_err(|error| error.to_string())
            .and_then(|parsed| validate_helper(Path::new("<helper>"), &parsed)
                .map_err(|error| error.to_string()))
            .unwrap_err()
            .contains(reason)
    );
}

#[test]
fn helper_case_cannot_disable_the_local_cost_map() {
    let case: HelperCase = serde_saphyr::from_str(
        "kind: helper\nname: reserved\ncall: {url_path: {function: segment, value: ., field_name: id}}\nenv: {LITELLM_LOCAL_MODEL_COST_MAP: 'False'}\ncompare: {ignore: []}\n",
    )
    .expect("helper case should parse");
    assert!(matches!(
        run_case(&GROUPS, &case, false).expect("helper should run"),
        HelperRun::Match
    ));
}

#[test]
fn proxy_policy_helper_renders_the_pick() {
    let call: HelperCall = serde_saphyr::from_str(
        "proxy_policy: {url: 'https://api.example.com', environment: {HTTPS_PROXY: 'http://u%40x:p@proxy.local:3128'}, aiohttp_trust_env: false, disable_aiohttp_trust_env: false}",
    )
    .unwrap();
    let output = serde_json::to_string(&invoke(&GROUPS, &call)).unwrap();
    let expected = r#"{"ok":true,"value":{"proxy":{"credentials":"u@x:p","host":"proxy.local","port":3128,"scheme":"http"}}}"#;
    assert_eq!(output, expected);
}

#[rstest::rstest]
#[case::too_long_get("get: {data: {}, default: null, key_path: ", "the path has 8193 bytes")]
#[case::too_long_delete("delete: {data: {}, path: ", "the path has 8193 bytes")]
fn dot_paths_adapter_reports_path_errors(#[case] prefix: &str, #[case] message: &str) {
    let source = format!("dot_paths: {{{prefix}{}}}}}", "a".repeat(8193));
    let call: HelperCall = serde_saphyr::from_str(&source).unwrap();
    let error = invoke(&GROUPS, &call).error.unwrap();
    assert_eq!(error.class, "PathError");
    assert!(error.message.starts_with(message));
}

fn group_operations(root: &Path) -> BTreeSet<String> {
    let entries: Vec<PathBuf> = fs::read_dir(root.join("src/helpers"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let group_files = entries
        .iter()
        .filter(|path| path.extension() == Some(OsStr::new("rs")))
        .filter(|path| {
            !["mod.rs", "tests.rs"]
                .iter()
                .any(|name| path.ends_with(name))
        })
        .cloned()
        .chain(
            entries
                .iter()
                .map(|path| path.join("lib.rs"))
                .filter(|path| path.is_file()),
        );
    group_files
        .flat_map(|path| {
            // operation names are the quoted strings on lines that open with a quote, several to a line
            fs::read_to_string(path)
                .unwrap()
                .lines()
                .filter(|line| line.trim_start().starts_with('"'))
                .flat_map(|line| {
                    line.split('"')
                        .skip(1)
                        .step_by(2)
                        .map(str::to_owned)
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn operations_txt_lists_exactly_the_registered_operations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let listed = fs::read_to_string(root.join("helpers/operations.txt")).unwrap();
    let python: BTreeSet<String> = listed.lines().map(str::to_owned).collect();
    assert_eq!(group_operations(root), python);
}

#[test]
fn a_null_success_value_is_sent_as_no_value_like_python_does() {
    let call: HelperCall = serde_saphyr::from_str(
        "database_url_policy: {function: unsupported_db_scheme, database_url: \"postgresql://u:p@host/db\"}",
    )
    .unwrap();
    let output = invoke(&GROUPS, &call);
    assert!(output.ok);
    assert_eq!(output.value, None);
}
