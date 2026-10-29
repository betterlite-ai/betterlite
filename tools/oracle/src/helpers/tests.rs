#![cfg(test)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::time::Duration;

use serde_json::json;

use super::answers::{Answers, Layer, key};
use crate::OracleError;
use crate::case::{HelperCase, parse_helper_case};

#[rstest::rstest]
#[case::hung_child("exec sleep 5", "helper timed out and was reaped")]
#[case::excess_output("head -c 1048577 /dev/zero", "helper output exceeded 1 MiB")]
#[case::failed_child("echo e >&2; exit 3", "helper exited with exit status: 3: e\n")]
fn helper_runner_tests_behavior(#[case] script: &str, #[case] expected: &str) {
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    let timeout = Duration::from_millis(300);
    let error = super::python::run_bounded(&mut command, Vec::new(), timeout).unwrap_err();
    assert_eq!(error.to_string(), expected);
    let unreaped = fs::read_to_string("/proc/thread-self/children").unwrap();
    assert_eq!(unreaped, "");
}

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
    assert!(parse_helper_case(&source).unwrap_err().contains(reason));
}

#[test]
fn management_helper_case_rejects_unknown_fields() {
    let source = "kind: helper\nname: invalid\ncall: {management_oracle: {domain: models, operation: {path_encoding: {function: segment, value: a, field_name: id, extra: true}}}}\nenv: {}\ncompare: {ignore: []}\n";
    let error = parse_helper_case(source).unwrap_err();
    assert!(error.contains("unknown field"));
}

#[rstest::rstest]
#[case::empty_name("''", "/a", "name cannot be empty")]
#[case::relative_pointer("a", "a", "JSON pointers")]
fn helper_cases_are_validated(#[case] name: &str, #[case] pointer: &str, #[case] reason: &str) {
    let source = format!(
        "kind: helper\nname: {name}\ncall: {{url_path: {{function: segment, value: a, field_name: id}}}}\nenv: {{}}\ncompare: {{ignore: [{{pointer: '{pointer}', why: w}}]}}\n"
    );
    assert!(parse_helper_case(&source).unwrap_err().contains(reason));
}

#[test]
fn helper_case_cannot_disable_the_local_cost_map() {
    let case: HelperCase = serde_saphyr::from_str(
        "kind: helper\nname: reserved\ncall: {url_path: {function: segment, value: ., field_name: id}}\nenv: {LITELLM_LOCAL_MODEL_COST_MAP: 'False'}\ncompare: {ignore: []}\n",
    )
    .expect("helper case should parse");
    let mut command = Command::new("true");
    super::python::configure_environment(&mut command, &case.env);
    let cost_map = command
        .get_envs()
        .find(|(name, _)| *name == "LITELLM_LOCAL_MODEL_COST_MAP");
    assert_eq!(
        cost_map.and_then(|(_, value)| value),
        Some(OsStr::new("True"))
    );
    assert!(matches!(
        super::run_case(&case, false).expect("helper should run"),
        super::HelperRun::Match
    ));
}

#[rstest::rstest]
#[case::at_limit(0x0010_0000, false)]
#[case::over_limit(0x0010_0001, true)]
fn helper_output_limit_is_enforced(#[case] size: usize, #[case] excess: bool) {
    let bytes = vec![b'x'; size];
    let (output, observed_excess) = super::python::read_limited(Cursor::new(bytes)).unwrap();
    assert_eq!(output.len(), 0x0010_0000.min(size));
    assert_eq!(observed_excess, excess);
}

#[test]
fn proxy_policy_helper_renders_the_pick() {
    let call: super::HelperCall = serde_saphyr::from_str(
        "proxy_policy: {url: 'https://api.example.com', environment: {HTTPS_PROXY: 'http://u%40x:p@proxy.local:3128'}, aiohttp_trust_env: false, disable_aiohttp_trust_env: false}",
    )
    .unwrap();
    let output = serde_json::to_string(&super::invoke(&call)).unwrap();
    let expected = r#"{"ok":true,"value":{"proxy":{"credentials":"u@x:p","host":"proxy.local","port":3128,"scheme":"http"}}}"#;
    assert_eq!(output, expected);
}

#[rstest::rstest]
#[case::too_long_get("get: {data: {}, default: null, key_path: ", "the path has 8193 bytes")]
#[case::too_long_delete("delete: {data: {}, path: ", "the path has 8193 bytes")]
fn dot_paths_adapter_reports_path_errors(#[case] prefix: &str, #[case] message: &str) {
    let source = format!("dot_paths: {{{prefix}{}}}}}", "a".repeat(8193));
    let call: super::HelperCall = serde_saphyr::from_str(&source).unwrap();
    let error = super::invoke(&call).error.unwrap();
    assert_eq!(error.class, "PathError");
    assert!(error.message.starts_with(message));
}

#[test]
fn dispatch_py_lists_exactly_the_registered_operations() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let dispatch = fs::read_to_string(root.join("helpers/dispatch.py")).unwrap();
    let (_, listed) = dispatch
        .split_once("OPERATIONS: frozenset[str] = frozenset(")
        .unwrap();
    let (operations, _) = listed.split_once(')').unwrap();
    let python: BTreeSet<String> = operations
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"')?.strip_suffix("\","))
        .map(str::to_owned)
        .collect();
    let group_files = fs::read_dir(root.join("src/helpers"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension() == Some(OsStr::new("rs")))
        .filter(|path| {
            !["answers.rs", "mod.rs", "python.rs", "tests.rs"]
                .iter()
                .any(|name| path.ends_with(name))
        });
    let rust: BTreeSet<String> = group_files
        .flat_map(|path| {
            // a group file quotes nothing but operation names, several to a line
            fs::read_to_string(path)
                .unwrap()
                .split('"')
                .skip(1)
                .step_by(2)
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(rust, python);
}

fn answer_scratch(name: &str) -> PathBuf {
    let scratch = env::temp_dir().join(format!("betterlite-answers-{}-{name}", process::id()));
    for keyed in [
        "tools/oracle/litellm/uv.lock",
        "tools/oracle/litellm/pyproject.toml",
        "tools/oracle/litellm/.python-version",
        "tools/oracle/helpers/dispatch.py",
    ] {
        let path = scratch.join(keyed);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, keyed).unwrap();
    }
    scratch
}

fn url_path_call() -> super::HelperCall {
    serde_saphyr::from_str("url_path: {function: segment, value: a/b, field_name: id}").unwrap()
}

#[test]
fn answer_cache_reads_the_shared_layer_first() {
    let root = answer_scratch("layers");
    let call = url_path_call();
    let case_env = BTreeMap::new();
    let name = format!("{}.json", key(&root, &call, &case_env).unwrap());
    let answers = Answers {
        shared: root.join("shared"),
        own: root.join("own"),
        write: Layer::Own,
    };
    fs::create_dir_all(&answers.shared).unwrap();
    fs::create_dir_all(&answers.own).unwrap();
    fs::write(
        answers.shared.join(&name),
        r#"{"ok":true,"value":"shared"}"#,
    )
    .unwrap();
    fs::write(answers.own.join(&name), r#"{"ok":true,"value":"own"}"#).unwrap();
    let shared = answers.answer(&root, &call, &case_env).unwrap();
    fs::remove_file(answers.shared.join(&name)).unwrap();
    let own = answers.answer(&root, &call, &case_env).unwrap();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(shared.value, Some(json!("shared")));
    assert_eq!(own.value, Some(json!("own")));
}

#[test]
fn answer_cache_key_follows_the_helper_module_the_pin_and_the_env() {
    let root = answer_scratch("key");
    let call = url_path_call();
    let case_env = BTreeMap::new();
    let mut keys = vec![key(&root, &call, &case_env).unwrap()];
    let module = root.join("tools/oracle/helpers/url_path.py");
    fs::write(&module, "first").unwrap();
    keys.push(key(&root, &call, &case_env).unwrap());
    fs::write(&module, "second").unwrap();
    keys.push(key(&root, &call, &case_env).unwrap());
    fs::write(root.join("tools/oracle/litellm/uv.lock"), "bumped").unwrap();
    keys.push(key(&root, &call, &case_env).unwrap());
    let named = BTreeMap::from([("A".to_owned(), "1".to_owned())]);
    keys.push(key(&root, &call, &named).unwrap());
    fs::remove_file(root.join("tools/oracle/helpers/dispatch.py")).unwrap();
    let missing = key(&root, &call, &case_env).unwrap_err();
    fs::remove_dir_all(&root).unwrap();
    assert_eq!(keys.iter().collect::<BTreeSet<_>>().len(), 5);
    assert!(matches!(missing, OracleError::Filesystem { .. }));
}

#[test]
fn answer_cache_stores_a_miss_in_the_write_layer_only() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let scratch = answer_scratch("miss");
    let answers = Answers {
        shared: scratch.join("shared"),
        own: scratch.join("own"),
        write: Layer::Shared,
    };
    let call = url_path_call();
    let case_env = BTreeMap::new();
    let output = answers.answer(&root, &call, &case_env).unwrap();
    let stored: Vec<OsString> = fs::read_dir(&answers.shared)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    let name = format!("{}.json", key(&root, &call, &case_env).unwrap());
    let own_written = answers.own.exists();
    fs::remove_dir_all(&scratch).unwrap();
    assert_eq!(stored, [OsString::from(name)]);
    assert!(!own_written);
    assert_eq!(output.value, super::invoke(&call).value);
}
